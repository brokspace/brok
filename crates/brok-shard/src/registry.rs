//! Installed shards and `shards.lock`.
//!
//! ```text
//! .brokspace/shards/
//! ├── shards.lock          provenance of every installed shard
//! ├── brok-agent/
//! │   ├── shard.json
//! │   └── bin/brok-agent
//! └── .staging/            temporary install area
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use brok_core::WorkspacePaths;
use brok_sdk::ShardManifest;
use serde::{Deserialize, Serialize};

/// Lock file name inside `.brokspace/shards/`.
pub const LOCK_FILE: &str = "shards.lock";
const LOCK_VERSION: u32 = 1;
pub(crate) const STAGING_DIR: &str = ".staging";

/// Where an installed shard came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ShardSource {
    /// A git repository.
    Git {
        /// Clone URL.
        url: String,
        /// Requested tag or branch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        /// Resolved commit.
        rev: String,
    },
    /// A local directory (shard development).
    Local {
        /// Absolute path.
        path: String,
    },
    /// Shipped alongside `brok` (the system agent).
    Builtin,
}

impl std::fmt::Display for ShardSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git { url, rev, .. } => write!(f, "{url}@{}", &rev[..rev.len().min(10)]),
            Self::Local { path } => write!(f, "{path}"),
            Self::Builtin => f.write_str("builtin"),
        }
    }
}

/// One entry of `shards.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    /// Installed version.
    pub version: semver::Version,
    /// Provenance.
    pub source: ShardSource,
    /// SHA-256 of the installed executable.
    pub binary_sha256: String,
    /// Install time (seconds since the Unix epoch).
    pub installed_at: u64,
}

impl LockEntry {
    pub(crate) fn new(version: semver::Version, source: ShardSource, binary_sha256: String) -> Self {
        let installed_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self { version, source, binary_sha256, installed_at }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LockFile {
    version: u32,
    shards: BTreeMap<String, LockEntry>,
}

/// A shard present in the registry.
#[derive(Debug, Clone)]
pub struct InstalledShard {
    /// Its manifest.
    pub manifest: ShardManifest,
    /// Its directory.
    pub dir: PathBuf,
    /// Provenance, when recorded.
    pub lock: Option<LockEntry>,
}

impl InstalledShard {
    /// Absolute path of the executable (`None` for skill-only shards).
    pub fn executable(&self) -> Option<PathBuf> {
        self.manifest.entrypoint_path(&self.dir)
    }

    /// Whether the shard has a process to run and its executable exists.
    pub fn is_runnable(&self) -> bool {
        self.executable().is_some_and(|path| path.is_file())
    }

    /// Absolute directories holding the shard's agent skills.
    pub fn skill_dirs(&self) -> Vec<PathBuf> {
        self.manifest.skills.iter().map(|dir| self.dir.join(dir)).collect()
    }
}

/// The shard registry of a workspace.
#[derive(Debug, Clone)]
pub struct Registry {
    dir: PathBuf,
}

impl Registry {
    /// Registry of the workspace described by `paths`.
    pub fn new(paths: &WorkspacePaths) -> Self {
        Self { dir: paths.shards.clone() }
    }

    /// Registry directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every installed shard, sorted by name. Broken entries are skipped
    /// with a warning.
    pub fn list(&self) -> Result<Vec<InstalledShard>> {
        let lock = self.read_lock();
        let mut shards = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Ok(shards) };
        for entry in entries.flatten() {
            let dir = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !dir.is_dir() || name.starts_with('.') {
                continue;
            }
            match ShardManifest::load_from_dir(&dir) {
                Ok(manifest) => {
                    let lock = lock.shards.get(&manifest.name).cloned();
                    shards.push(InstalledShard { manifest, dir, lock });
                }
                Err(e) => tracing::warn!(shard = %name, error = %e, "ignoring broken shard"),
            }
        }
        shards.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        Ok(shards)
    }

    /// Looks a shard up by name.
    pub fn get(&self, name: &str) -> Result<Option<InstalledShard>> {
        Ok(self.list()?.into_iter().find(|s| s.manifest.name == name))
    }

    /// Uninstalls a shard. Returns `false` when it was not installed.
    pub fn remove(&self, name: &str) -> Result<bool> {
        if !brok_sdk::manifest::is_valid_name(name) {
            return Ok(false);
        }
        let dir = self.dir.join(name);
        let existed = dir.exists();
        if existed {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        let mut lock = self.read_lock();
        let locked = lock.shards.remove(name).is_some();
        self.write_lock(&lock)?;
        Ok(existed || locked)
    }

    /// Moves a fully prepared shard directory into place and records it.
    pub(crate) fn commit(&self, prepared: &Path, manifest: ShardManifest, entry: LockEntry) -> Result<InstalledShard> {
        let target = self.dir.join(&manifest.name);
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| format!("replacing {}", target.display()))?;
        }
        brok_core::fsutil::move_dir(prepared, &target)
            .with_context(|| format!("installing into {}", target.display()))?;
        let mut lock = self.read_lock();
        lock.shards.insert(manifest.name.clone(), entry.clone());
        self.write_lock(&lock)?;
        Ok(InstalledShard { manifest, dir: target, lock: Some(entry) })
    }

    pub(crate) fn staging_dir(&self) -> PathBuf {
        self.dir.join(STAGING_DIR)
    }

    fn read_lock(&self) -> LockFile {
        std::fs::read(self.dir.join(LOCK_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(LockFile { version: LOCK_VERSION, shards: BTreeMap::new() })
    }

    fn write_lock(&self, lock: &LockFile) -> Result<()> {
        let json = serde_json::to_vec_pretty(&LockFile { version: LOCK_VERSION, shards: lock.shards.clone() })?;
        brok_core::fsutil::write_atomic(&self.dir.join(LOCK_FILE), &json)?;
        Ok(())
    }
}
