//! Workspace discovery and the `.brokspace/` directory layout.

use std::path::{Path, PathBuf};

use brok_sdk::Endpoint;

use crate::config::BrokConfig;
use crate::error::{CoreError, Result};

/// Name of the per-workspace state directory.
pub const BROKSPACE_DIR: &str = ".brokspace";
/// Name of the configuration file inside [`BROKSPACE_DIR`].
pub const CONFIG_FILE: &str = "brok.yml";

/// Every well-known location inside `.brokspace/`.
///
/// ```text
/// .brokspace/
/// ├── brok.yml        workspace configuration
/// ├── cache/models/   ONNX weights and tokenizers
/// ├── databases/
/// │   ├── cold/       root workspace partition
/// │   └── hot/<id>/   one micro-database per git submodule
/// ├── shards/         installed shards + shards.lock
/// ├── run/            runtime state (socket, tokens, pid)
/// └── logs/           shard logs
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePaths {
    /// Workspace root (the directory containing `.brokspace/`).
    pub root: PathBuf,
    /// `.brokspace/`
    pub brokspace: PathBuf,
    /// `.brokspace/brok.yml`
    pub config_file: PathBuf,
    /// `.brokspace/cache/`
    pub cache: PathBuf,
    /// `.brokspace/cache/models/`
    pub models: PathBuf,
    /// `.brokspace/databases/`
    pub databases: PathBuf,
    /// `.brokspace/shards/`
    pub shards: PathBuf,
    /// `.brokspace/run/`
    pub run: PathBuf,
    /// `.brokspace/logs/`
    pub logs: PathBuf,
}

impl WorkspacePaths {
    /// Computes the layout for a workspace root.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let brokspace = root.join(BROKSPACE_DIR);
        Self {
            config_file: brokspace.join(CONFIG_FILE),
            cache: brokspace.join("cache"),
            models: brokspace.join("cache").join("models"),
            databases: brokspace.join("databases"),
            shards: brokspace.join("shards"),
            run: brokspace.join("run"),
            logs: brokspace.join("logs"),
            brokspace,
            root,
        }
    }

    /// Directories created by `brok init`.
    pub fn scaffold_dirs(&self) -> [&Path; 6] {
        [&self.brokspace, &self.models, &self.databases, &self.shards, &self.run, &self.logs]
    }

    /// Directory of an installed shard.
    pub fn shard_dir(&self, name: &str) -> PathBuf {
        self.shards.join(name)
    }

    /// Token file used by CLI commands to talk to `brok run`.
    pub fn operator_token(&self) -> PathBuf {
        self.run.join("operator.token")
    }

    /// PID file of the running daemon.
    pub fn pid_file(&self) -> PathBuf {
        self.run.join("brok.pid")
    }
}

/// An opened workspace: its root, layout and parsed configuration.
#[derive(Debug, Clone)]
pub struct Workspace {
    paths: WorkspacePaths,
    config: BrokConfig,
}

impl Workspace {
    /// Walks up from `start` until a directory containing
    /// `.brokspace/brok.yml` is found, then opens it.
    pub fn discover(start: &Path) -> Result<Self> {
        let start = dunce_canonicalize(start)?;
        let root = start
            .ancestors()
            .find(|dir| dir.join(BROKSPACE_DIR).join(CONFIG_FILE).is_file())
            .ok_or_else(|| CoreError::NotAWorkspace(start.clone()))?;
        Self::open(root)
    }

    /// Opens the workspace rooted exactly at `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let paths = WorkspacePaths::new(dunce_canonicalize(root)?);
        if !paths.config_file.is_file() {
            return Err(CoreError::NotAWorkspace(paths.root));
        }
        let config = BrokConfig::load(&paths.config_file)?;
        Ok(Self { paths, config })
    }

    /// Workspace root.
    pub fn root(&self) -> &Path {
        &self.paths.root
    }

    /// Directory layout.
    pub fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    /// Parsed configuration.
    pub fn config(&self) -> &BrokConfig {
        &self.config
    }

    /// Replaces and persists the configuration.
    pub fn save_config(&mut self, config: BrokConfig) -> Result<()> {
        config.validate()?;
        config.save(&self.paths.config_file)?;
        self.config = config;
        Ok(())
    }

    /// IPC endpoint of this workspace's bus.
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::for_workspace(self.root())
    }

    /// Human friendly workspace name (the root directory name).
    pub fn name(&self) -> String {
        self.root().file_name().map_or_else(|| "workspace".into(), |n| n.to_string_lossy().into_owned())
    }
}

/// Canonicalizes without the `\\?\` prefix Windows adds, which breaks many
/// tools (git included) and makes paths ugly in messages.
pub fn dunce_canonicalize(path: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path).map_err(|e| CoreError::io(path, e))?;
    #[cfg(windows)]
    {
        let raw = canonical.to_string_lossy();
        if let Some(stripped) = raw.strip_prefix(r"\\?\")
            && !stripped.starts_with("UNC\\")
        {
            return Ok(PathBuf::from(stripped));
        }
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        let paths = WorkspacePaths::new(dir.path());
        std::fs::create_dir_all(&paths.brokspace).unwrap();
        std::fs::write(&paths.config_file, "version: 1\n").unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();

        let ws = Workspace::discover(&nested).unwrap();
        assert_eq!(ws.root(), dunce_canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn discover_fails_outside_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Workspace::discover(dir.path()), Err(CoreError::NotAWorkspace(_))));
    }
}
