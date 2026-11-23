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

impl LockFile {
    fn empty() -> Self {
        Self { version: LOCK_VERSION, shards: BTreeMap::new() }
    }
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
        // Listing only reads: an unreadable lock must not hide the shards,
        // it just leaves them without provenance.
        let lock = self.read_lock().unwrap_or_else(|e| {
            tracing::warn!(error = format!("{e:#}"), "ignoring unreadable shards.lock");
            LockFile::empty()
        });
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
        let mut lock = self.read_lock()?;
        let dir = self.dir.join(name);
        let existed = dir.exists();
        if existed {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        let locked = lock.shards.remove(name).is_some();
        self.write_lock(&lock)?;
        Ok(existed || locked)
    }

    /// Moves a fully prepared shard directory into place and records it.
    pub(crate) fn commit(&self, prepared: &Path, manifest: ShardManifest, entry: LockEntry) -> Result<InstalledShard> {
        // Read the lock first: a lock that cannot be read or written back
        // safely must stop the install before anything is replaced.
        let mut lock = self.read_lock()?;
        let target = self.dir.join(&manifest.name);
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| format!("replacing {}", target.display()))?;
        }
        brok_core::fsutil::move_dir(prepared, &target)
            .with_context(|| format!("installing into {}", target.display()))?;
        lock.shards.insert(manifest.name.clone(), entry.clone());
        self.write_lock(&lock)?;
        Ok(InstalledShard { manifest, dir: target, lock: Some(entry) })
    }

    pub(crate) fn staging_dir(&self) -> PathBuf {
        self.dir.join(STAGING_DIR)
    }

    /// A missing lock is an empty one. A lock that cannot be parsed, or that
    /// a newer brok wrote, is an error: writing over it would destroy the
    /// pinned hashes it holds.
    fn read_lock(&self) -> Result<LockFile> {
        let path = self.dir.join(LOCK_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LockFile::empty()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        // The version alone first, so a newer format reports "upgrade", not a parse error.
        #[derive(Deserialize)]
        struct Header {
            version: u32,
        }
        let header: Header = serde_json::from_slice(&bytes).with_context(|| {
            format!("{} is not a valid lock file; fix or remove it (nothing was changed)", path.display())
        })?;
        anyhow::ensure!(
            header.version <= LOCK_VERSION,
            "{} has lock version {}, but this brok understands up to {LOCK_VERSION}: upgrade brok (nothing was changed)",
            path.display(),
            header.version
        );
        serde_json::from_slice(&bytes).with_context(|| {
            format!("{} is not a valid lock file; fix or remove it (nothing was changed)", path.display())
        })
    }

    fn write_lock(&self, lock: &LockFile) -> Result<()> {
        let json = serde_json::to_vec_pretty(&LockFile { version: LOCK_VERSION, shards: lock.shards.clone() })?;
        brok_core::fsutil::write_atomic(&self.dir.join(LOCK_FILE), &json)?;
        Ok(())
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    fn registry() -> (tempfile::TempDir, Registry) {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Registry { dir: tmp.path().to_path_buf() };
        (tmp, registry)
    }

    fn put(registry: &Registry, content: &str) {
        std::fs::write(registry.dir.join(LOCK_FILE), content).unwrap();
    }

    #[test]
    fn a_missing_lock_is_empty() {
        let (_tmp, registry) = registry();
        assert!(registry.read_lock().unwrap().shards.is_empty());
        assert!(!registry.remove("nothing").unwrap());
    }

    #[test]
    fn a_damaged_lock_is_an_error_and_is_left_alone() {
        let (_tmp, registry) = registry();
        put(&registry, "{ not json <<<<<<< HEAD");
        let err = format!("{:#}", registry.read_lock().unwrap_err());
        assert!(err.contains("not a valid lock file"), "{err}");
        assert!(registry.remove("brok-agent").is_err());
        assert_eq!(std::fs::read_to_string(registry.dir.join(LOCK_FILE)).unwrap(), "{ not json <<<<<<< HEAD");
        // Listing is read-only and keeps working.
        assert!(registry.list().unwrap().is_empty());
    }

    #[test]
    fn a_newer_lock_asks_for_an_upgrade() {
        let (_tmp, registry) = registry();
        let newer = format!("{{\"version\": {}, \"shards\": {{}}, \"extra\": 1}}", LOCK_VERSION + 1);
        put(&registry, &newer);
        let err = format!("{:#}", registry.read_lock().unwrap_err());
        assert!(err.contains("upgrade brok"), "{err}");
        assert!(registry.remove("brok-agent").is_err());
        assert_eq!(std::fs::read_to_string(registry.dir.join(LOCK_FILE)).unwrap(), newer);
    }

    #[test]
    fn a_valid_lock_with_extra_fields_still_reads() {
        let (_tmp, registry) = registry();
        put(&registry, "{\"version\": 1, \"shards\": {}, \"future\": true}");
        assert!(registry.read_lock().is_ok());
    }
}
