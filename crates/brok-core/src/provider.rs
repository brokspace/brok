//! Source-control providers behind a capability-based contract (ADR 0015).
//!
//! Git is the default and most complete provider, wrapping [`crate::git`]
//! without changing how it runs. A provider declares what it can do;
//! anything else returns [`ProviderError::Unsupported`], never an empty
//! success. Revision identifiers are opaque strings kept as the provider
//! returns them. Provider configuration holds credential *references*, never
//! secret values.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::error::CoreError;

/// Version of the [`SourceControl`] contract.
pub const CONTRACT_VERSION: u32 = 1;

/// Id of the built-in Git provider.
pub const GIT: &str = "git";

/// Something a provider may or may not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// Working tree status and changed paths.
    Status,
    /// Diffs and change details.
    Diff,
    /// Revision history.
    History,
    /// Change notifications.
    Watch,
    /// Submodule-like nested sources the provider tracks itself.
    Submodules,
    /// Several working trees of one repository.
    Worktrees,
}

impl Capability {
    /// Stable lowercase name, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Status => "status",
            Capability::Diff => "diff",
            Capability::History => "history",
            Capability::Watch => "watch",
            Capability::Submodules => "submodules",
            Capability::Worktrees => "worktrees",
        }
    }
}

/// Identity and capabilities of a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInfo {
    /// Unique id (`git`).
    pub id: String,
    /// [`CONTRACT_VERSION`] the provider implements.
    pub contract_version: u32,
    /// What it supports beyond the required operations.
    pub capabilities: BTreeSet<Capability>,
}

impl ProviderInfo {
    /// Whether `capability` is supported.
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// An opaque provider-native revision identifier (a commit id for Git).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision(pub String);

/// How a path changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Not tracked yet.
    Untracked,
    /// Added to the tracked set.
    Added,
    /// Content changed.
    Modified,
    /// Removed.
    Deleted,
    /// Moved (the path is the new one).
    Renamed,
    /// Anything else (conflicts, type changes).
    Other,
}

/// A changed path, relative to the repository root, with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The path.
    pub path: String,
    /// What happened to it.
    pub kind: ChangeKind,
}

/// Errors of a provider.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The provider does not support the operation.
    #[error("source control provider `{provider}` does not support `{}`", .capability.as_str())]
    Unsupported {
        /// Provider id.
        provider: String,
        /// The missing capability.
        capability: Capability,
    },
    /// The operation failed.
    #[error(transparent)]
    Failed(#[from] CoreError),
}

/// The source-control contract. `is_repository` and `nested_repositories`
/// are required; everything else is gated by [`Capability`].
pub trait SourceControl: Send + Sync {
    /// Identity and capabilities.
    fn info(&self) -> &ProviderInfo;

    /// Whether `dir` is inside a repository of this provider.
    fn is_repository(&self, dir: &Path) -> bool;

    /// Independent repositories nested below `root`, up to `max_depth` levels.
    fn nested_repositories(&self, root: &Path, max_depth: usize) -> Vec<String>;

    /// Current revision, or `None` when there is none yet.
    fn head_revision(&self, _root: &Path) -> Result<Option<Revision>, ProviderError> {
        Err(self.unsupported(Capability::History))
    }

    /// Paths with uncommitted changes.
    fn status(&self, _root: &Path) -> Result<Vec<Change>, ProviderError> {
        Err(self.unsupported(Capability::Status))
    }

    /// Workspace relative paths of submodule-like sources.
    fn submodules(&self, _root: &Path) -> Result<Vec<String>, ProviderError> {
        Err(self.unsupported(Capability::Submodules))
    }

    /// The error for an operation this provider cannot do.
    fn unsupported(&self, capability: Capability) -> ProviderError {
        ProviderError::Unsupported { provider: self.info().id.clone(), capability }
    }
}

/// The built-in Git provider.
#[derive(Debug, Clone)]
pub struct GitProvider {
    info: ProviderInfo,
}

impl Default for GitProvider {
    fn default() -> Self {
        Self {
            info: ProviderInfo {
                id: GIT.to_owned(),
                contract_version: CONTRACT_VERSION,
                capabilities: BTreeSet::from([Capability::Status, Capability::History, Capability::Submodules]),
            },
        }
    }
}

impl SourceControl for GitProvider {
    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    fn is_repository(&self, dir: &Path) -> bool {
        crate::git::is_repository(dir)
    }

    fn nested_repositories(&self, root: &Path, max_depth: usize) -> Vec<String> {
        crate::git::nested_repositories(root, max_depth)
    }

    fn head_revision(&self, root: &Path) -> Result<Option<Revision>, ProviderError> {
        // A repository without commits has no HEAD: not an error.
        match crate::git::run(root, &["rev-parse", "--verify", "--quiet", "HEAD"]) {
            Ok(id) if !id.is_empty() => Ok(Some(Revision(id))),
            Ok(_) => Ok(None),
            Err(CoreError::Git { message, .. }) if message.is_empty() => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn status(&self, root: &Path) -> Result<Vec<Change>, ProviderError> {
        let out = crate::git::run_raw(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
        Ok(parse_status(&out))
    }

    fn submodules(&self, root: &Path) -> Result<Vec<String>, ProviderError> {
        Ok(crate::git::submodules(root))
    }
}

/// Parses `git status --porcelain=v1 -z`: `XY path\0`, and for a rename
/// `XY new\0old\0`.
fn parse_status(output: &str) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut fields = output.split('\0').filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let (Some(code), Some(path)) = (field.get(..2), field.get(3..)) else { continue };
        let mut chars = code.chars();
        let (x, y) = (chars.next().unwrap_or(' '), chars.next().unwrap_or(' '));
        let kind = match (x, y) {
            ('?', '?') => ChangeKind::Untracked,
            ('R' | 'C', _) | (_, 'R' | 'C') => {
                fields.next(); // the old path
                ChangeKind::Renamed
            }
            ('A', _) => ChangeKind::Added,
            ('D', _) | (_, 'D') => ChangeKind::Deleted,
            ('M', _) | (_, 'M') => ChangeKind::Modified,
            _ => ChangeKind::Other,
        };
        changes.push(Change { path: path.to_owned(), kind });
    }
    changes
}

/// Providers by id.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<String, Arc<dyn SourceControl>>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.providers.keys()).finish()
    }
}

impl ProviderRegistry {
    /// A registry holding the built-in Git provider.
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry.providers.insert(GIT.to_owned(), Arc::new(GitProvider::default()));
        registry
    }

    /// Registers a provider. Ids are unique and the contract version must match.
    pub fn register(&mut self, provider: Arc<dyn SourceControl>) -> Result<(), RegistryError> {
        let info = provider.info();
        if info.id.is_empty() || !info.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(RegistryError::InvalidId(info.id.clone()));
        }
        if info.contract_version != CONTRACT_VERSION {
            return Err(RegistryError::ContractVersion {
                id: info.id.clone(),
                found: info.contract_version,
                expected: CONTRACT_VERSION,
            });
        }
        if self.providers.contains_key(&info.id) {
            return Err(RegistryError::Duplicate(info.id.clone()));
        }
        self.providers.insert(info.id.clone(), provider);
        Ok(())
    }

    /// Looks a provider up by id.
    pub fn get(&self, id: &str) -> Result<Arc<dyn SourceControl>, RegistryError> {
        self.providers.get(id).cloned().ok_or_else(|| RegistryError::Unknown {
            id: id.to_owned(),
            known: self.providers.keys().cloned().collect::<Vec<_>>().join(", "),
        })
    }

    /// Identity and capabilities of every provider, sorted by id.
    pub fn list(&self) -> Vec<ProviderInfo> {
        self.providers.values().map(|p| p.info().clone()).collect()
    }
}

/// Errors of the registry.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    /// A provider with this id is already registered.
    #[error("a source control provider with id `{0}` is already registered")]
    Duplicate(String),
    /// The id is not lowercase letters, digits and dashes.
    #[error("invalid provider id `{0}` (use lowercase letters, digits and dashes)")]
    InvalidId(String),
    /// The provider implements another contract version.
    #[error("provider `{id}` implements contract v{found}, this brok supports v{expected}")]
    ContractVersion {
        /// Provider id.
        id: String,
        /// Version it reports.
        found: u32,
        /// Version supported.
        expected: u32,
    },
    /// No provider with this id.
    #[error("unknown source control provider `{id}` (known: {known})")]
    Unknown {
        /// Requested id.
        id: String,
        /// Registered ids.
        known: String,
    },
}

/// The provider used when none is configured: Git.
pub fn default_source_control() -> Arc<dyn SourceControl> {
    static DEFAULT: OnceLock<Arc<dyn SourceControl>> = OnceLock::new();
    DEFAULT.get_or_init(|| Arc::new(GitProvider::default())).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider with no optional capability, to exercise `Unsupported`.
    struct Minimal(ProviderInfo);

    impl Minimal {
        fn new(id: &str, version: u32) -> Arc<Self> {
            Arc::new(Self(ProviderInfo { id: id.into(), contract_version: version, capabilities: BTreeSet::new() }))
        }
    }

    impl SourceControl for Minimal {
        fn info(&self) -> &ProviderInfo {
            &self.0
        }
        fn is_repository(&self, _dir: &Path) -> bool {
            false
        }
        fn nested_repositories(&self, _root: &Path, _max_depth: usize) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn unsupported_operations_are_explicit() {
        let provider = Minimal::new("minimal", CONTRACT_VERSION);
        let err = provider.status(Path::new(".")).unwrap_err();
        assert!(matches!(err, ProviderError::Unsupported { capability: Capability::Status, .. }));
        assert_eq!(err.to_string(), "source control provider `minimal` does not support `status`");
        assert!(provider.submodules(Path::new(".")).is_err());
        assert!(provider.head_revision(Path::new(".")).is_err());
    }

    #[test]
    fn registry_rules() {
        let mut registry = ProviderRegistry::builtin();
        assert_eq!(registry.list().len(), 1);
        assert!(registry.get(GIT).unwrap().info().supports(Capability::Status));
        registry.register(Minimal::new("minimal", CONTRACT_VERSION)).unwrap();
        assert_eq!(
            registry.register(Minimal::new("minimal", CONTRACT_VERSION)),
            Err(RegistryError::Duplicate("minimal".into()))
        );
        assert_eq!(
            registry.register(Minimal::new("git", CONTRACT_VERSION)),
            Err(RegistryError::Duplicate("git".into()))
        );
        assert!(matches!(registry.register(Minimal::new("Bad Id", 1)), Err(RegistryError::InvalidId(_))));
        assert!(matches!(registry.register(Minimal::new("old", 0)), Err(RegistryError::ContractVersion { .. })));
        let err = registry.get("nope").err().unwrap().to_string();
        assert!(err.contains("known: git, minimal"), "{err}");
        let ids: Vec<_> = registry.list().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["git", "minimal"]);
    }

    #[test]
    fn parses_porcelain_status() {
        let out = " M src/a.rs\0A  new.rs\0?? scratch.txt\0 D gone.rs\0R  moved.rs\0old.rs\0";
        let changes = parse_status(out);
        let got: Vec<_> = changes.iter().map(|c| (c.path.as_str(), c.kind)).collect();
        assert_eq!(
            got,
            [
                ("src/a.rs", ChangeKind::Modified),
                ("new.rs", ChangeKind::Added),
                ("scratch.txt", ChangeKind::Untracked),
                ("gone.rs", ChangeKind::Deleted),
                ("moved.rs", ChangeKind::Renamed),
            ]
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
            .args(["-c", "protocol.file.allow=always"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn git_provider_matches_the_git_functions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let provider = GitProvider::default();
        assert!(!provider.is_repository(root));

        git(root, &["init", "-q"]);
        assert!(provider.is_repository(root));
        // No commits yet: no HEAD, and that is not an error.
        assert_eq!(provider.head_revision(root).unwrap(), None);
        assert!(provider.status(root).unwrap().is_empty());

        std::fs::write(root.join("a.txt"), "one").unwrap();
        assert_eq!(provider.status(root).unwrap(), [Change { path: "a.txt".into(), kind: ChangeKind::Untracked }]);
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "first"]);
        let head = provider.head_revision(root).unwrap().expect("a commit");
        assert_eq!(head.0.len(), 40);
        assert!(provider.status(root).unwrap().is_empty());
        std::fs::write(root.join("a.txt"), "two").unwrap();
        assert_eq!(provider.status(root).unwrap(), [Change { path: "a.txt".into(), kind: ChangeKind::Modified }]);
    }

    #[test]
    fn git_provider_finds_nested_repositories_and_submodules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("meta");
        let lib = tmp.path().join("lib");
        for dir in [&root, &lib] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q"]);
        }
        std::fs::write(lib.join("l.txt"), "x").unwrap();
        git(&lib, &["add", "."]);
        git(&lib, &["commit", "-q", "-m", "lib"]);
        std::fs::create_dir_all(root.join("tools/cli")).unwrap();
        git(&root.join("tools/cli"), &["init", "-q"]);
        git(&root, &["submodule", "add", "-q", lib.to_str().unwrap(), "vendor-lib"]);

        let provider = GitProvider::default();
        assert_eq!(provider.submodules(&root).unwrap(), ["vendor-lib"]);
        assert_eq!(provider.nested_repositories(&root, 4), ["tools/cli", "vendor-lib"]);
        // The same answers as the functions the rest of brok used before.
        assert_eq!(provider.submodules(&root).unwrap(), crate::git::submodules(&root));
        assert_eq!(provider.nested_repositories(&root, 4), crate::git::nested_repositories(&root, 4));
    }
}
