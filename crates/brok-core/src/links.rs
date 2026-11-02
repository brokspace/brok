//! Linked workspaces (`workspace.links` in `brok.yml`).
//!
//! A link names another workspace this one depends on, such as the
//! `checkout` service a `user` service calls. Its code and memories are
//! searched with this workspace's, so answers know about both.
//!
//! - `path`: a directory on this machine. When it is a Brokspace workspace,
//!   its own index and memories are used (read only); otherwise brok indexes
//!   it into `.brokspace/links/<name>/state/`.
//! - `git`: a repository brok clones (shallow) into
//!   `.brokspace/links/<name>/src/` and indexes into `state/`; `brok link
//!   sync` pulls and re-indexes it.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{BrokConfig, LinkConfig};
use crate::error::{CoreError, Result};
use crate::workspace::{BROKSPACE_DIR, CONFIG_FILE, Workspace};

/// How a link is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    /// A directory on this machine.
    Local,
    /// A git repository cloned by brok.
    Git,
}

/// Whose index a link uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkIndex {
    /// The linked directory is a Brokspace workspace: its own index.
    Own,
    /// Indexed by this workspace, in `.brokspace/links/<name>/state/`.
    Managed,
}

/// A link resolved against the workspace that declares it.
#[derive(Debug, Clone)]
pub struct Link {
    /// Name in `workspace.links`.
    pub name: String,
    /// As configured.
    pub config: LinkConfig,
    /// Local or git.
    pub kind: LinkKind,
    /// Directory of the linked sources.
    pub root: PathBuf,
    /// `.brokspace/links/<name>/`
    pub dir: PathBuf,
}

impl Link {
    /// Resolves the link `name` of `workspace`.
    pub fn resolve(workspace: &Workspace, name: &str, config: &LinkConfig) -> Self {
        let dir = workspace.paths().links.join(name);
        let (kind, root) = match (&config.path, &config.git) {
            (Some(path), _) => (LinkKind::Local, absolute(workspace.root(), path)),
            _ => (LinkKind::Git, dir.join("src")),
        };
        Self { name: name.to_owned(), config: config.clone(), kind, root, dir }
    }

    /// Every link of `workspace`, sorted by name.
    pub fn all(workspace: &Workspace) -> Vec<Self> {
        workspace.config().workspace.links.iter().map(|(name, config)| Self::resolve(workspace, name, config)).collect()
    }

    /// Whether the sources are on disk.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// Whose index the link uses.
    pub fn index(&self) -> LinkIndex {
        if self.kind == LinkKind::Local && self.root.join(BROKSPACE_DIR).join(CONFIG_FILE).is_file() {
            LinkIndex::Own
        } else {
            LinkIndex::Managed
        }
    }

    /// `.brokspace/links/<name>/state/`: the index brok keeps for it.
    pub fn state_dir(&self) -> PathBuf {
        self.dir.join("state")
    }

    /// The workspace to index or search: the linked workspace itself, or a
    /// detached one over its sources configured like `parent` (without its
    /// links, so links never chain).
    pub fn workspace(&self, parent: &BrokConfig) -> Result<Workspace> {
        if !self.exists() {
            return Err(CoreError::Config(match self.kind {
                LinkKind::Local => format!("link `{}`: {} does not exist", self.name, self.root.display()),
                LinkKind::Git => format!("link `{}` is not cloned yet (run `brok link sync {}`)", self.name, self.name),
            }));
        }
        match self.index() {
            LinkIndex::Own => Workspace::open(&self.root),
            LinkIndex::Managed => {
                let mut config = parent.clone();
                config.workspace.links.clear();
                config.shards.clear();
                Workspace::detached(&self.root, &self.state_dir(), config)
            }
        }
    }

    /// Clones a git link, or fetches and checks out its latest commit.
    /// Local links are left alone. Returns the checked out commit.
    pub fn fetch(&self) -> Result<Option<String>> {
        let Some(url) = self.config.git.as_deref().filter(|_| self.kind == LinkKind::Git) else { return Ok(None) };
        std::fs::create_dir_all(&self.dir).map_err(|e| CoreError::io(&self.dir, e))?;
        if self.root.join(".git").exists() {
            let branch = self.config.branch.as_deref().unwrap_or("HEAD");
            crate::git::run(&self.root, &["fetch", "--quiet", "--depth", "1", "origin", branch])?;
            crate::git::run(&self.root, &["reset", "--quiet", "--hard", "FETCH_HEAD"])?;
        } else {
            let _ = std::fs::remove_dir_all(&self.root); // a clone that failed half way
            let mut args = vec!["clone", "--quiet", "--depth", "1", "--no-tags"];
            if let Some(branch) = self.config.branch.as_deref() {
                args.extend(["--branch", branch]);
            }
            args.extend([url, "src"]);
            crate::git::run(&self.dir, &args)?;
        }
        crate::git::run(&self.root, &["rev-parse", "--short", "HEAD"]).map(Some)
    }

    /// `path` as it is best shown from `from` (the declaring workspace's
    /// root): relative when the two share a parent, absolute otherwise.
    pub fn display_root(&self, from: &Path) -> String {
        relative_to(from, &self.root).unwrap_or_else(|| self.root.to_string_lossy().replace('\\', "/"))
    }
}

/// `path` (absolute, `~/...` or relative to `root`) made absolute and tidy.
fn absolute(root: &Path, path: &str) -> PathBuf {
    let path = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir().map_or_else(|| PathBuf::from(path), |home| home.join(rest)),
        None => PathBuf::from(path),
    };
    let joined = if path.is_absolute() { path } else { root.join(path) };
    crate::workspace::dunce_canonicalize(&joined).unwrap_or(joined)
}

/// `target` relative to `base` (`../checkout/src`), `/` separated, when both
/// are absolute paths on the same drive.
pub fn relative_to(base: &Path, target: &Path) -> Option<String> {
    use std::path::Component;
    let base: Vec<Component<'_>> = base.components().collect();
    let target: Vec<Component<'_>> = target.components().collect();
    if base.first() != target.first() {
        return None; // different drive or root
    }
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_owned(), base.len() - common).collect();
    parts.extend(target[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    Some(if parts.is_empty() { ".".into() } else { parts.join("/") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::WorkspacePaths;

    fn workspace(root: &Path, yaml: &str) -> Workspace {
        let paths = WorkspacePaths::new(root);
        std::fs::create_dir_all(&paths.brokspace).unwrap();
        std::fs::write(&paths.config_file, yaml).unwrap();
        Workspace::open(root).unwrap()
    }

    #[test]
    fn local_and_git_links() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user");
        let checkout = tmp.path().join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        let ws = workspace(
            &user,
            "workspace:\n  links:\n    checkout:\n      path: ../checkout\n    payments:\n      git: https://example.com/p.git\n",
        );
        let links = Link::all(&ws);
        assert_eq!(links.len(), 2);
        let local = &links[0];
        assert_eq!((local.name.as_str(), local.kind), ("checkout", LinkKind::Local));
        assert!(local.exists());
        assert_eq!(local.index(), LinkIndex::Managed);
        assert_eq!(local.display_root(ws.root()), "../checkout");

        // A detached workspace: sources in checkout/, state under user/.brokspace/links/.
        let detached = local.workspace(ws.config()).unwrap();
        assert_eq!(detached.root(), crate::workspace::dunce_canonicalize(&checkout).unwrap());
        assert!(detached.paths().databases.starts_with(ws.paths().links.join("checkout").join("state")));
        assert!(detached.config().workspace.links.is_empty());

        // Becoming a Brokspace workspace switches to its own index.
        workspace(&checkout, "version: 1\n");
        assert_eq!(local.index(), LinkIndex::Own);

        let git = &links[1];
        assert_eq!(git.kind, LinkKind::Git);
        assert!(!git.exists());
        assert!(git.workspace(ws.config()).unwrap_err().to_string().contains("brok link sync payments"));
    }

    #[test]
    fn relative_paths() {
        let base = std::env::temp_dir().join("a").join("user");
        assert_eq!(relative_to(&base, &base.parent().unwrap().join("checkout")).unwrap(), "../checkout");
        assert_eq!(relative_to(&base, &base.join(".brokspace").join("links")).unwrap(), ".brokspace/links");
        assert_eq!(relative_to(&base, &base).unwrap(), ".");
    }
}
