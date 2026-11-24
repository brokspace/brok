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
    /// Directory of the repository, absolute or relative to this workspace's
    /// root. Exactly one of `path` and `git`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Git URL brok clones (shallow, read only) into
    /// `.brokspace/members/<id>/src` and indexes into `state/`. Never put
    /// credentials in it: use your Git credential helper or SSH keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// Branch of `git` (the remote's default branch when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
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
        match (member.path.trim().is_empty(), &member.git) {
            (true, None) => errors.push(format!("workspace.members.{id}: `path` or `git` is required")),
            (false, Some(_)) => errors.push(format!("workspace.members.{id}: use either `path` or `git`, not both")),
            (_, Some(url)) => {
                if !looks_like_git_url(url) {
                    errors.push(format!("workspace.members.{id}.git: `{url}` is not a git URL"));
                } else if url_has_credentials(url) {
                    errors.push(format!(
                        "workspace.members.{id}.git: do not put credentials in the URL (use a credential helper or SSH keys)"
                    ));
                }
            }
            _ => {}
        }
        if member.branch.is_some() && member.git.is_none() {
            errors.push(format!("workspace.members.{id}: `branch` only applies to `git` members"));
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

/// Whether `target` names a git repository rather than a directory.
pub fn looks_like_git_url(target: &str) -> bool {
    target.contains("://") || target.starts_with("git@") || target.ends_with(".git")
}

/// Whether a URL carries a password or token (`https://user:token@host/...`,
/// `https://token@host/...`). `git@host:` (ssh) is a username, not a secret.
pub fn url_has_credentials(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else { return false };
    let authority = rest.split('/').next().unwrap_or_default();
    match authority.rsplit_once('@') {
        Some((userinfo, _)) => userinfo.contains(':') || !matches!(userinfo, "git" | "ssh" | "hg"),
        None => false,
    }
}

/// Where a member stands on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberState {
    /// The directory is a repository.
    Repository,
    /// The directory exists but is not a repository (a plain folder).
    Directory,
    /// The directory does not exist (moved or deleted).
    Missing,
    /// A `git` member that has not been cloned yet (`brok workspace sync`).
    NotCloned,
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
        let root = if config.git.is_some() {
            Self::managed_dir(workspace_root, id).join("src")
        } else {
            let raw = Path::new(&config.path);
            let joined = if raw.is_absolute() { raw.to_path_buf() } else { workspace_root.join(raw) };
            dunce_canonicalize(&joined).unwrap_or(joined)
        };
        Self {
            id: id.to_owned(),
            name: config.name.clone().unwrap_or_else(|| id.to_owned()),
            root,
            config: config.clone(),
        }
    }

    /// `.brokspace/members/<id>/`: the clone and index brok keeps for a `git` member.
    pub fn managed_dir(workspace_root: &Path, id: &str) -> PathBuf {
        workspace_root.join(crate::workspace::BROKSPACE_DIR).join("members").join(id)
    }

    /// Whether brok clones this member (a `git` source) instead of reading a directory.
    pub fn is_git(&self) -> bool {
        self.config.git.is_some()
    }

    /// Every member of `workspace`, sorted by id.
    pub fn all(workspace: &Workspace) -> Vec<Self> {
        workspace.config().workspace.members.iter().map(|(id, c)| Self::resolve(workspace.root(), id, c)).collect()
    }

    /// Whether the member is a repository, a plain folder or missing.
    pub fn state(&self) -> MemberState {
        if !self.root.is_dir() {
            if self.is_git() { MemberState::NotCloned } else { MemberState::Missing }
        } else if default_source_control().is_repository(&self.root) {
            MemberState::Repository
        } else {
            MemberState::Directory
        }
    }

    /// Whether the member is itself a Brokspace workspace (has its own index).
    pub fn is_workspace(&self) -> bool {
        // A clone's own `.brokspace` (if it commits one) is not used: brok
        // keeps the index of a `git` member itself.
        !self.is_git() && self.root.join(".brokspace").join("brok.yml").is_file()
    }

    /// A `git` member as a [`Link`], which already knows how to clone, fetch
    /// and open a detached workspace over a clone. `None` for a directory.
    pub fn as_link(&self, workspace_root: &Path) -> Option<crate::links::Link> {
        let git = self.config.git.clone()?;
        let config =
            crate::config::LinkConfig { git: Some(git), branch: self.config.branch.clone(), ..Default::default() };
        Some(crate::links::Link {
            name: self.id.clone(),
            config,
            kind: crate::links::LinkKind::Git,
            root: self.root.clone(),
            dir: Self::managed_dir(workspace_root, &self.id),
        })
    }

    /// The workspace to search for this member, read only: its own Brokspace
    /// (a directory that has one), or the index brok keeps for a cloned `git`
    /// member. `None` when there is nothing to search (a plain directory, or
    /// a `git` member not cloned yet).
    pub fn searchable(&self, workspace: &Workspace) -> Option<Result<Workspace>> {
        if let Some(link) = self.as_link(workspace.root()) {
            return link.exists().then(|| link.workspace(workspace.config()));
        }
        self.is_workspace().then(|| Workspace::open(&self.root))
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
    members.insert(id.clone(), MemberConfig { path: stored, git: None, branch: None, name, description });
    Ok(Registration::Added { id })
}

/// The repository name at the end of a git URL (`https://host/org/api.git` and
/// `git@host:org/api` both give `api`).
fn repository_name(url: &str) -> String {
    let tail = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or_default();
    tail.strip_suffix(".git").unwrap_or(tail).to_owned()
}

/// The same repository however it is spelled (trailing `/` and `.git`).
fn same_url(a: &str, b: &str) -> bool {
    let norm = |u: &str| u.trim().trim_end_matches('/').trim_end_matches(".git").to_ascii_lowercase();
    norm(a) == norm(b)
}

/// Registers a git repository brok clones (`brok workspace sync`) as a
/// member. Registering the same URL again is a no-op; credentials in the URL
/// are refused, since `brok.yml` is committed.
pub fn register_git(
    members: &mut BTreeMap<String, MemberConfig>,
    url: &str,
    branch: Option<String>,
    id: Option<&str>,
    name: Option<String>,
    description: Option<String>,
) -> Result<Registration> {
    let fail = |message: String| CoreError::Config(message);
    let url = url.trim();
    if !looks_like_git_url(url) {
        return Err(fail(format!("`{url}` is not a git URL")));
    }
    if url_has_credentials(url) {
        return Err(fail(
            "do not put credentials in the URL: brok.yml is committed. Use your Git credential helper or SSH keys"
                .into(),
        ));
    }
    for (existing_id, existing) in members.iter() {
        if existing.git.as_deref().is_some_and(|u| same_url(u, url)) && existing.branch == branch {
            if let Some(wanted) = id
                && wanted != existing_id
            {
                return Err(fail(format!("{url} is already registered as `{existing_id}`")));
            }
            return Ok(Registration::Existing { id: existing_id.clone() });
        }
    }
    let id = id.map_or_else(|| slug(&repository_name(url)), str::to_owned);
    if !is_member_id(&id) {
        return Err(fail(format!(
            "`{id}` is not a valid member id (lowercase letters, digits and `-`, starting with a letter); pass --id"
        )));
    }
    if members.contains_key(&id) {
        return Err(fail(format!("member id `{id}` is already used by another repository; pass --id")));
    }
    members
        .insert(id.clone(), MemberConfig { path: String::new(), git: Some(url.to_owned()), branch, name, description });
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

    #[test]
    fn registers_git_members_idempotently_without_credentials() {
        let mut members = BTreeMap::new();
        let added =
            register_git(&mut members, "https://github.com/acme/Payment-API.git", None, None, None, None).unwrap();
        assert_eq!(added, Registration::Added { id: "payment-api".into() });
        // The same repository however it is spelled.
        let again = register_git(&mut members, "https://github.com/acme/payment-api/", None, None, None, None).unwrap();
        assert_eq!(again, Registration::Existing { id: "payment-api".into() });
        assert_eq!(members.len(), 1);
        // A branch is a different member source, so it needs its own id.
        assert!(
            register_git(&mut members, "git@github.com:acme/payment-api.git", Some("next".into()), None, None, None)
                .is_err()
        );
        let other = register_git(
            &mut members,
            "git@github.com:acme/payment-api.git",
            Some("next".into()),
            Some("payments-next"),
            None,
            None,
        );
        assert_eq!(other.unwrap(), Registration::Added { id: "payments-next".into() });

        for secret in ["https://user:hunter2@example.com/a/b.git", "https://ghp_token@example.com/a/b.git"] {
            let err = register_git(&mut members, secret, None, None, None, None).unwrap_err().to_string();
            assert!(err.contains("credentials"), "{err}");
            assert!(
                !err.contains("hunter2") && !err.contains("ghp_token"),
                "the error must not repeat the secret: {err}"
            );
        }
        assert!(register_git(&mut members, "not a url", None, None, None, None).is_err());
        assert!(!url_has_credentials("git@github.com:acme/x.git"));
        assert!(!url_has_credentials("ssh://git@example.com/acme/x.git"));
        assert!(!url_has_credentials("https://example.com/acme/x.git"));
    }

    #[test]
    fn a_member_is_a_path_or_a_git_url_not_both() {
        let mut members = BTreeMap::new();
        members.insert(
            "both".into(),
            MemberConfig { path: "../a".into(), git: Some("https://example.com/a.git".into()), ..Default::default() },
        );
        members.insert("neither".into(), MemberConfig::default());
        members.insert(
            "branchy".into(),
            MemberConfig { path: "../b".into(), branch: Some("x".into()), ..Default::default() },
        );
        members.insert(
            "secret".into(),
            MemberConfig { git: Some("https://u:p@example.com/a.git".into()), ..Default::default() },
        );
        let errors = validate(&members, &[]).join("\n");
        for expected in ["not both", "`path` or `git` is required", "`branch` only applies", "credentials"] {
            assert!(errors.contains(expected), "{expected} missing from:\n{errors}");
        }
    }

    #[test]
    fn a_git_member_lives_under_the_workspace_until_cloned() {
        let dir = tempfile::tempdir().unwrap();
        let config = MemberConfig { git: Some("https://example.com/acme/api.git".into()), ..Default::default() };
        let member = Member::resolve(dir.path(), "api", &config);
        assert!(member.is_git());
        assert!(member.root.ends_with(std::path::Path::new(".brokspace/members/api/src")));
        assert_eq!(member.state(), MemberState::NotCloned);
        assert!(!member.is_workspace());
        assert_eq!(member.as_link(dir.path()).unwrap().dir, Member::managed_dir(dir.path(), "api"));
    }
}
