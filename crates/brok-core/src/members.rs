//! Meta Workspace members (ADR 0014): independent repositories registered in
//! `workspace.members` of `brok.yml`, and the typed relationships between
//! them.
//!
//! A meta workspace is an ordinary workspace that lists members. Nothing
//! changes without that section. Members keep their own source control,
//! configuration and index; this module only records identity, location and
//! relationships, and never writes inside a member.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::provider::default_source_control;
use crate::workspace::Workspace;
use crate::workspace::dunce_canonicalize;

/// Directories of the child repositories `discover` never looks into.
const SKIP_DIRS: &[&str] = &[".git", ".brokspace", "node_modules", "target", "dist", "build", "vendor"];

/// `workspace.members.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemberConfig {
    /// Directory of the repository, absolute or relative to this workspace's root.
    pub path: String,
    /// Display name (the id when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What it is and how it relates to the rest (given to the LLM).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// How two members relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// `from` needs `to` to build or run.
    DependsOn,
    /// `from` calls an API `to` publishes.
    ConsumesApi,
    /// `from` publishes an API `to` consumes.
    PublishesApi,
    /// `from` uses `to` as a library.
    UsesLibrary,
    /// `from` is deployed together with `to`.
    DeploysWith,
    /// `from` holds tests of `to`.
    Tests,
    /// `from` and `to` share a contract (schema, protocol).
    SharesContractWith,
}

impl RelationKind {
    /// The name used in `brok.yml` and on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            RelationKind::DependsOn => "depends_on",
            RelationKind::ConsumesApi => "consumes_api",
            RelationKind::PublishesApi => "publishes_api",
            RelationKind::UsesLibrary => "uses_library",
            RelationKind::DeploysWith => "deploys_with",
            RelationKind::Tests => "tests",
            RelationKind::SharesContractWith => "shares_contract_with",
        }
    }

    /// Parses the name used in `brok.yml`.
    pub fn parse(name: &str) -> Option<Self> {
        [
            RelationKind::DependsOn,
            RelationKind::ConsumesApi,
            RelationKind::PublishesApi,
            RelationKind::UsesLibrary,
            RelationKind::DeploysWith,
            RelationKind::Tests,
            RelationKind::SharesContractWith,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == name)
    }
}

/// `workspace.relationships[]`: a typed edge between two member ids.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    /// Source member id.
    pub from: String,
    /// Kind of relationship.
    pub kind: RelationKind,
    /// Target member id.
    pub to: String,
}

/// Whether `id` can identify a member: lowercase kebab-case, at most 63 characters.
pub fn is_member_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.ends_with('-')
}

/// Turns a directory name into a member id (`My API_v2` becomes `my-api-v2`).
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    let mut id = if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("repo-{trimmed}")
    } else {
        trimmed.to_owned()
    };
    id.truncate(63);
    id.trim_end_matches('-').to_owned()
}

/// Semantic checks of the members and relationships of a configuration.
pub fn validate(members: &BTreeMap<String, MemberConfig>, relationships: &[Relationship]) -> Vec<String> {
    let mut errors = Vec::new();
    for (id, member) in members {
        if !is_member_id(id) {
            errors.push(format!(
                "workspace.members: `{id}` is not a valid id (lowercase letters, digits and `-`, starting with a letter)"
            ));
        }
        if member.path.trim().is_empty() {
            errors.push(format!("workspace.members.{id}: `path` is required"));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for edge in relationships {
        for end in [&edge.from, &edge.to] {
            if !members.contains_key(end) {
                errors.push(format!("workspace.relationships: `{end}` is not a member"));
            }
        }
        if edge.from == edge.to {
            errors.push(format!("workspace.relationships: `{}` cannot relate to itself", edge.from));
        }
        if !seen.insert(edge) {
            errors.push(format!(
                "workspace.relationships: `{} {} {}` is listed twice",
                edge.from,
                edge.kind.as_str(),
                edge.to
            ));
        }
    }
    errors
}

/// Where a member stands on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberState {
    /// The directory is a repository.
    Repository,
    /// The directory exists but is not a repository (a plain folder).
    Directory,
    /// The directory does not exist (not cloned here, moved or deleted).
    Missing,
}

/// A member resolved against the workspace that declares it.
#[derive(Debug, Clone)]
pub struct Member {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Absolute directory.
    pub root: PathBuf,
    /// As configured.
    pub config: MemberConfig,
}

impl Member {
    /// Resolves member `id`.
    pub fn resolve(workspace_root: &Path, id: &str, config: &MemberConfig) -> Self {
        let raw = Path::new(&config.path);
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { workspace_root.join(raw) };
        let root = dunce_canonicalize(&joined).unwrap_or(joined);
        Self {
            id: id.to_owned(),
            name: config.name.clone().unwrap_or_else(|| id.to_owned()),
            root,
            config: config.clone(),
        }
    }

    /// Every member of `workspace`, sorted by id.
    pub fn all(workspace: &Workspace) -> Vec<Self> {
        workspace.config().workspace.members.iter().map(|(id, c)| Self::resolve(workspace.root(), id, c)).collect()
    }

    /// Whether the member is a repository, a plain folder or missing.
    pub fn state(&self) -> MemberState {
        if !self.root.is_dir() {
            MemberState::Missing
        } else if default_source_control().is_repository(&self.root) {
            MemberState::Repository
        } else {
            MemberState::Directory
        }
    }

    /// Whether the member is itself a Brokspace workspace (has its own index).
    pub fn is_workspace(&self) -> bool {
        self.root.join(".brokspace").join("brok.yml").is_file()
    }
}

/// The result of registering a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// A new member was added.
    Added {
        /// Its id.
        id: String,
    },
    /// The directory was already registered under this id: nothing changed.
    Existing {
        /// Its id.
        id: String,
    },
}

/// Registers `target` as a member of the workspace rooted at `workspace_root`,
/// editing `members` only. Registering the same directory again is a no-op.
///
/// Refused: the workspace's own root or `.brokspace`, a directory inside the
/// workspace that is already indexed as a nested repository or submodule
/// (it is part of this workspace, not a member), a directory that contains or
/// is inside another member (indexes would overlap), and an id used by
/// another directory.
pub fn register(
    workspace_root: &Path,
    members: &mut BTreeMap<String, MemberConfig>,
    target: &Path,
    id: Option<&str>,
    name: Option<String>,
    description: Option<String>,
) -> Result<Registration> {
    let fail = |message: String| CoreError::Config(message);
    let joined = if target.is_absolute() { target.to_path_buf() } else { workspace_root.join(target) };
    if !joined.is_dir() {
        return Err(fail(format!("{} is not a directory", joined.display())));
    }
    let dir = dunce_canonicalize(&joined)?;
    let root = dunce_canonicalize(workspace_root)?;
    if dir == root {
        return Err(fail("a workspace cannot be a member of itself".into()));
    }
    if dir.starts_with(root.join(".brokspace")) {
        return Err(fail("`.brokspace` cannot be a member".into()));
    }
    if dir.starts_with(&root) {
        let provider = default_source_control();
        let mut inside = provider.submodules(&root).unwrap_or_default();
        inside.extend(provider.nested_repositories(&root, 4));
        let relative = crate::fsutil::to_workspace_relative(&root, &dir).unwrap_or_default();
        if inside.contains(&relative) {
            return Err(fail(format!(
                "{relative} is a nested repository or submodule: it is already indexed as part of this workspace"
            )));
        }
    }

    for (existing_id, existing) in members.iter() {
        let other = Member::resolve(workspace_root, existing_id, existing).root;
        if other == dir {
            if let Some(wanted) = id
                && wanted != existing_id
            {
                return Err(fail(format!("{} is already registered as `{existing_id}`", dir.display())));
            }
            return Ok(Registration::Existing { id: existing_id.clone() });
        }
        if other.starts_with(&dir) || dir.starts_with(&other) {
            return Err(fail(format!(
                "{} overlaps member `{existing_id}` ({}): members must not contain each other",
                dir.display(),
                other.display()
            )));
        }
    }

    let id = match id {
        Some(id) => id.to_owned(),
        None => slug(&dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
    };
    if !is_member_id(&id) {
        return Err(fail(format!(
            "`{id}` is not a valid member id (lowercase letters, digits and `-`, starting with a letter); pass --id"
        )));
    }
    if members.contains_key(&id) {
        return Err(fail(format!("member id `{id}` is already used by another directory; pass --id")));
    }
    // Relative to the workspace (`../payment-api`) so a committed brok.yml works
    // on every machine laid out the same way; absolute only across drives.
    let stored = crate::links::relative_to(&root, &dir).unwrap_or_else(|| dir.to_string_lossy().replace('\\', "/"));
    members.insert(id.clone(), MemberConfig { path: stored, name, description });
    Ok(Registration::Added { id })
}

/// Git repositories directly inside `dir`, as candidate members, skipping the
/// ones already registered. Does not recurse.
pub fn discover(workspace_root: &Path, dir: &Path, members: &BTreeMap<String, MemberConfig>) -> Vec<PathBuf> {
    let registered: Vec<PathBuf> = members.iter().map(|(id, m)| Member::resolve(workspace_root, id, m).root).collect();
    let provider = default_source_control();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str())
        })
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(".git").exists() && provider.is_repository(path))
        .filter_map(|path| dunce_canonicalize(&path).ok())
        .filter(|path| !registered.contains(path))
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let status = std::process::Command::new("git").arg("-C").arg(dir).args(["init", "-q"]).status().unwrap();
        assert!(status.success());
    }

    fn add(
        root: &Path,
        members: &mut BTreeMap<String, MemberConfig>,
        target: &str,
        id: Option<&str>,
    ) -> Result<Registration> {
        register(root, members, Path::new(target), id, None, None)
    }

    #[test]
    fn ids_and_slugs() {
        assert!(is_member_id("payment-api") && is_member_id("a1"));
        assert!(
            !is_member_id("Payment")
                && !is_member_id("1api")
                && !is_member_id("a_b")
                && !is_member_id("a-")
                && !is_member_id("")
        );
        assert_eq!(slug("My API_v2"), "my-api-v2");
        assert_eq!(slug("42-things"), "repo-42-things");
        assert_eq!(slug("--x--"), "x");
        assert!(is_member_id(&slug(&"a".repeat(100))));
    }

    #[test]
    fn registers_independent_repositories_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join("product");
        std::fs::create_dir_all(&meta).unwrap();
        repo(&tmp.path().join("payment-api"));
        repo(&tmp.path().join("Web Frontend"));
        let mut members = BTreeMap::new();

        assert_eq!(
            add(&meta, &mut members, "../payment-api", None).unwrap(),
            Registration::Added { id: "payment-api".into() }
        );
        assert_eq!(
            add(&meta, &mut members, "../Web Frontend", None).unwrap(),
            Registration::Added { id: "web-frontend".into() }
        );
        // Again, by another spelling of the same path: no change.
        let before = members.clone();
        assert_eq!(
            add(&meta, &mut members, "../payment-api/../payment-api", None).unwrap(),
            Registration::Existing { id: "payment-api".into() }
        );
        assert_eq!(members, before);
        // The same directory under another id is refused, not duplicated.
        assert!(add(&meta, &mut members, "../payment-api", Some("other")).is_err());
        // Another directory cannot take a used id.
        repo(&tmp.path().join("other-api"));
        assert!(add(&meta, &mut members, "../other-api", Some("payment-api")).is_err());
        assert_eq!(members.len(), 2);
        assert!(!tmp.path().join("payment-api/.brokspace").exists(), "a member is never written to");
    }

    #[test]
    fn refuses_itself_overlaps_and_partitions() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join("meta");
        repo(&meta);
        repo(&meta.join("tools/cli"));
        repo(&tmp.path().join("svc"));
        let mut members = BTreeMap::new();
        assert!(add(&meta, &mut members, ".", None).is_err(), "itself");
        assert!(add(&meta, &mut members, "tools/cli", None).is_err(), "already a nested partition");
        add(&meta, &mut members, "../svc", None).unwrap();
        std::fs::create_dir_all(tmp.path().join("svc/sub")).unwrap();
        assert!(add(&meta, &mut members, "../svc/sub", Some("sub")).is_err(), "inside a member");
        assert!(add(&meta, &mut members, "..", Some("all")).is_err(), "contains a member");
        assert!(add(&meta, &mut members, "../nowhere", None).is_err(), "missing");
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn paths_are_stored_relative_to_the_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join("meta");
        std::fs::create_dir_all(meta.join("services/auth")).unwrap();
        let mut members = BTreeMap::new();
        add(&meta, &mut members, "services/auth", None).unwrap();
        assert_eq!(members["auth"].path, "services/auth");
        repo(&tmp.path().join("beside"));
        add(&meta, &mut members, "../beside", None).unwrap();
        assert_eq!(members["beside"].path, "../beside");
        let member = Member::resolve(&meta, "auth", &members["auth"]);
        assert_eq!(member.state(), MemberState::Directory);
        assert!(!member.is_workspace());
        std::fs::remove_dir_all(meta.join("services/auth")).unwrap();
        assert_eq!(Member::resolve(&meta, "auth", &members["auth"]).state(), MemberState::Missing);
    }

    #[test]
    fn validates_relationships() {
        let mut members = BTreeMap::new();
        for id in ["api", "web"] {
            members.insert(id.to_owned(), MemberConfig { path: format!("../{id}"), ..Default::default() });
        }
        let edge = |from: &str, kind, to: &str| Relationship { from: from.into(), kind, to: to.into() };
        assert!(validate(&members, &[edge("web", RelationKind::ConsumesApi, "api")]).is_empty());
        let errors = validate(
            &members,
            &[
                edge("web", RelationKind::DependsOn, "ghost"),
                edge("api", RelationKind::Tests, "api"),
                edge("web", RelationKind::UsesLibrary, "api"),
                edge("web", RelationKind::UsesLibrary, "api"),
            ],
        );
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert_eq!(RelationKind::parse("shares_contract_with"), Some(RelationKind::SharesContractWith));
        assert_eq!(RelationKind::parse("friends"), None);
    }

    #[test]
    fn discovers_unregistered_repositories() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join("meta");
        std::fs::create_dir_all(&meta).unwrap();
        repo(&tmp.path().join("a"));
        repo(&tmp.path().join("b"));
        repo(&tmp.path().join(".hidden"));
        std::fs::create_dir_all(tmp.path().join("plain")).unwrap();
        let mut members = BTreeMap::new();
        add(&meta, &mut members, "../a", None).unwrap();
        let found = discover(&meta, tmp.path(), &members);
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["b"]);
    }
}
