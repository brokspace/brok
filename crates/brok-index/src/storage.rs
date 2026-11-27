//! What a workspace costs on disk, and which of it is left over. Read only:
//! it opens no index and takes no lock, and it deletes nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use brok_core::{PartitionMap, Tier, Workspace};
use serde::{Deserialize, Serialize};

use crate::embed::ModelFiles;
use crate::partition_index::{BM25_FILE, MANIFEST_FILE};
use crate::schema::Precision;
use crate::store::arrow::CURRENT;

/// A temporary file younger than this may still be being written.
const TEMPORARY_AGE: Duration = Duration::from_secs(60 * 60);

/// Folders directly under `.brokspace/` and the category they count as.
/// Anything else there is `other`.
const CATEGORIES: [(&str, &str); 8] = [
    ("databases", "index"),
    ("links", "linked workspaces"),
    ("cache", "model cache"),
    ("memory", "private memory"),
    ("logs", "logs"),
    ("metrics", "metrics"),
    ("shards", "shards"),
    ("shards-data", "shard data"),
];

/// Disk usage of one kind of data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Category {
    /// What it holds.
    pub name: String,
    /// Where it is, workspace relative.
    pub path: String,
    /// File sizes added up.
    pub bytes: u64,
    /// Number of files.
    pub files: u64,
}

/// Disk usage of one partition's index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartitionUsage {
    /// Partition id.
    pub id: String,
    /// Its database folder, workspace relative.
    pub path: String,
    /// Everything in that folder.
    pub bytes: u64,
    /// Keyword (BM25) index.
    pub keyword_bytes: u64,
    /// File manifest.
    pub manifest_bytes: u64,
    /// Chunk store: text and vectors together.
    pub store_bytes: u64,
    /// Chunks recorded in the manifest.
    pub chunks: u64,
    /// Estimated size of the vectors inside the store (`chunks` x dimensions
    /// x 2 or 4 bytes); `None` without semantic search.
    pub vector_bytes_estimate: Option<u64>,
    /// No part of the workspace maps to this folder now.
    pub orphaned: bool,
}

/// Space that holds nothing in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leftover {
    /// Why it looks unused.
    pub reason: String,
    /// Workspace relative path.
    pub path: String,
    /// Its size.
    pub bytes: u64,
}

/// The embedding model folder shared by all workspaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedModels {
    /// Absolute path.
    pub path: PathBuf,
    /// Its size.
    pub bytes: u64,
}

/// The storage report of a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorageReport {
    /// Everything this workspace keeps: `.brokspace/` and the shared memories.
    pub total_bytes: u64,
    /// Per kind of data; empty kinds are left out.
    pub categories: Vec<Category>,
    /// Per partition database.
    pub partitions: Vec<PartitionUsage>,
    /// Models outside the workspace, not part of the total.
    pub shared_models: Option<SharedModels>,
    /// Candidates for cleanup, largest first.
    pub leftovers: Vec<Leftover>,
    /// Sum of the leftovers.
    pub leftover_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct ManifestSummary {
    /// 1 for `Float32` vectors, 2 for `Float16`.
    #[serde(default)]
    version: u32,
    #[serde(default)]
    embedding: Option<Dimensions>,
    #[serde(default)]
    files: BTreeMap<String, Chunks>,
}

#[derive(Debug, Deserialize)]
struct Dimensions {
    dims: u64,
}

#[derive(Debug, Deserialize)]
struct Chunks {
    #[serde(default)]
    chunks: u64,
}

/// A file below some folder: its path relative to it, and its metadata.
struct Entry {
    path: PathBuf,
    bytes: u64,
    modified: Option<SystemTime>,
}

/// Every file below `dir`. Links are not followed, and what cannot be read
/// (or disappears while we look) is left out.
fn files_below(dir: &Path) -> Vec<Entry> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            // `DirEntry::metadata` does not follow symbolic links.
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                let Ok(path) = entry.path().strip_prefix(dir).map(Path::to_path_buf) else { continue };
                files.push(Entry { path, bytes: metadata.len(), modified: metadata.modified().ok() });
            }
        }
    }
    files
}

/// `path` relative to the workspace with `/` separators.
fn shown(root: &Path, path: &Path) -> String {
    brok_core::fsutil::to_workspace_relative(root, path).unwrap_or_else(|| path.display().to_string())
}

/// Measures `workspace`.
pub fn report(workspace: &Workspace) -> StorageReport {
    measure(workspace, SystemTime::now())
}

fn measure(workspace: &Workspace, now: SystemTime) -> StorageReport {
    let root = workspace.root();
    let paths = workspace.paths();
    let state = files_below(&paths.brokspace);

    let mut categories: BTreeMap<(&str, PathBuf), (u64, u64)> = BTreeMap::new();
    for entry in &state {
        let first = entry.path.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let known = CATEGORIES.iter().find(|(folder, _)| first.as_deref() == Some(*folder));
        let key = match known {
            Some((folder, name)) => (*name, paths.brokspace.join(folder)),
            None => ("other", paths.brokspace.clone()),
        };
        let usage = categories.entry(key).or_default();
        usage.0 += entry.bytes;
        usage.1 += 1;
    }
    for entry in files_below(&paths.shared_memory) {
        let usage = categories.entry(("shared memory", paths.shared_memory.clone())).or_default();
        usage.0 += entry.bytes;
        usage.1 += 1;
    }
    let mut categories: Vec<Category> = categories
        .into_iter()
        .map(|((name, path), (bytes, files))| Category { name: name.into(), path: shown(root, &path), bytes, files })
        .collect();
    categories.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));

    let current = PartitionMap::discover(root, workspace.config().workspace.submodules);
    let known: BTreeSet<PathBuf> = current.iter().map(|p| p.database_dir(paths)).collect();
    let mut folders = vec![("root".to_owned(), paths.databases.join("cold"))];
    if let Some(root_partition) = current.iter().find(|p| p.tier == Tier::Cold) {
        folders[0].0.clone_from(&root_partition.id);
    }
    if let Ok(hot) = fs::read_dir(paths.databases.join("hot")) {
        for entry in hot.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
            folders.push((entry.file_name().to_string_lossy().into_owned(), entry.path()));
        }
    }
    folders.sort();

    let mut leftovers = Vec::new();
    let mut partitions = Vec::new();
    let mut orphans = Vec::new();
    for (id, dir) in folders {
        let files = files_below(&dir);
        if files.is_empty() {
            continue;
        }
        let size_of = |name: &str| files.iter().filter(|f| f.path == Path::new(name)).map(|f| f.bytes).sum::<u64>();
        let bytes = files.iter().map(|f| f.bytes).sum();
        let (keyword_bytes, manifest_bytes) = (size_of(BM25_FILE), size_of(MANIFEST_FILE));
        let manifest: Option<ManifestSummary> =
            fs::read(dir.join(MANIFEST_FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let chunks = manifest.as_ref().map_or(0, |m| m.files.values().map(|f| f.chunks).sum());
        let vector_bytes_estimate = manifest.as_ref().and_then(|m| {
            let precision = if m.version >= 2 { Precision::Half } else { Precision::Single };
            m.embedding.as_ref().map(|e| chunks * e.dims * precision.bytes())
        });
        let orphaned = !known.contains(&dir);
        if orphaned {
            leftovers.push(Leftover {
                reason: "index of a repository that is not part of the workspace now".into(),
                path: shown(root, &dir),
                bytes,
            });
            orphans.push(dir.clone());
        }
        partitions.push(PartitionUsage {
            id,
            path: shown(root, &dir),
            bytes,
            keyword_bytes,
            manifest_bytes,
            store_bytes: bytes - keyword_bytes - manifest_bytes,
            chunks,
            vector_bytes_estimate,
            orphaned,
        });
    }

    let mut live_generations: BTreeMap<PathBuf, Option<u64>> = BTreeMap::new();
    for entry in &state {
        let path = paths.brokspace.join(&entry.path);
        // Shards own their folders, and an orphan is already counted whole.
        if entry.path.starts_with("shards")
            || entry.path.starts_with("shards-data")
            || orphans.iter().any(|dir| path.starts_with(dir))
        {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let old = entry.modified.and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= TEMPORARY_AGE);
        let reason = if name.ends_with(".tmp") {
            old.then_some("temporary file left behind")
        } else if let Some(generation) =
            name.strip_prefix("chunks-").and_then(|n| n.strip_suffix(".arrow")).and_then(|n| n.parse::<u64>().ok())
        {
            let dir = path.parent().unwrap_or(&paths.brokspace).to_path_buf();
            let live = *live_generations
                .entry(dir)
                .or_insert_with_key(|dir| fs::read_to_string(dir.join(CURRENT)).ok()?.trim().parse().ok());
            // A newer generation than the live one is being published.
            live.is_some_and(|live| generation < live).then_some("replaced chunk store generation")
        } else {
            None
        };
        if let Some(reason) = reason {
            leftovers.push(Leftover { reason: reason.into(), path: shown(root, &path), bytes: entry.bytes });
        }
    }
    leftovers.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));

    let shared_models = ModelFiles::locate(&paths.models, &workspace.config().embeddings)
        .ok()
        .and_then(|files| files.dir.parent().map(Path::to_path_buf))
        .filter(|dir| !dir.starts_with(&paths.brokspace))
        .map(|path| SharedModels { bytes: files_below(&path).iter().map(|f| f.bytes).sum(), path })
        .filter(|models| models.bytes > 0);

    StorageReport {
        total_bytes: categories.iter().map(|c| c.bytes).sum(),
        categories,
        partitions,
        shared_models,
        leftover_bytes: leftovers.iter().map(|l| l.bytes).sum(),
        leftovers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, bytes: usize) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), ".brokspace/brok.yml", 0);
        fs::write(dir.path().join(".brokspace/brok.yml"), "version: 1\n").unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    /// Every file with its content, to prove nothing changed.
    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut all: Vec<_> =
            files_below(root).into_iter().map(|f| (f.path.clone(), fs::read(root.join(f.path)).unwrap())).collect();
        all.sort();
        all
    }

    #[test]
    fn adds_up_categories_and_partitions_without_touching_anything() {
        let (dir, workspace) = workspace();
        let root = dir.path();
        let manifest = r#"{"embedding":{"model":"m","dims":768},"files":{"a.rs":{"chunks":2},"b.rs":{"chunks":1}}}"#;
        write(root, ".brokspace/databases/cold/files.json", 0);
        fs::write(root.join(".brokspace/databases/cold/files.json"), manifest).unwrap();
        write(root, ".brokspace/databases/cold/bm25.bin", 100);
        write(root, ".brokspace/databases/cold/chunks.lance/data/part", 900);
        write(root, ".brokspace/logs/brok.log", 50);
        write(root, ".brokspace-memory/decision.md", 30);
        write(root, "src/main.rs", 4_000);
        let before = snapshot(root);

        let report = report(&workspace);

        assert_eq!(snapshot(root), before);
        let of = |name: &str| report.categories.iter().find(|c| c.name == name).map(|c| c.bytes);
        assert_eq!(of("index"), Some(manifest.len() as u64 + 1_000));
        assert_eq!((of("logs"), of("shared memory")), (Some(50), Some(30)));
        assert_eq!(report.total_bytes, report.categories.iter().map(|c| c.bytes).sum::<u64>());
        assert!(report.total_bytes < 4_000, "sources are not counted");
        let partition = &report.partitions[0];
        assert_eq!((partition.keyword_bytes, partition.store_bytes, partition.chunks), (100, 900, 3));
        assert_eq!(partition.vector_bytes_estimate, Some(3 * 768 * 4));
        assert!(!partition.orphaned);
        assert!(report.leftovers.is_empty(), "{:?}", report.leftovers);

        let half = manifest.replacen('{', r#"{"version":2,"#, 1);
        fs::write(root.join(".brokspace/databases/cold/files.json"), half).unwrap();
        assert_eq!(super::report(&workspace).partitions[0].vector_bytes_estimate, Some(3 * 768 * 2));
    }

    #[test]
    fn finds_leftovers_and_leaves_live_files_alone() {
        let (dir, workspace) = workspace();
        let root = dir.path();
        write(root, ".brokspace/databases/cold/chunks-00000001.arrow", 10);
        write(root, ".brokspace/databases/cold/chunks-00000002.arrow", 20);
        write(root, ".brokspace/databases/cold/chunks-00000003.arrow", 40);
        fs::write(root.join(".brokspace/databases/cold/CURRENT"), "2").unwrap();
        write(root, ".brokspace/databases/cold/.files.json.99.tmp", 7);
        write(root, ".brokspace/databases/hot/gone/bm25.bin", 300);
        write(root, ".brokspace/databases/hot/gone/.bm25.bin.99.tmp", 5);
        write(root, ".brokspace/shards-data/other/.cache.tmp", 1_000);

        let fresh = report(&workspace);
        let paths = |report: &StorageReport| report.leftovers.iter().map(|l| l.path.clone()).collect::<Vec<_>>();
        // A temporary file just written may be in use.
        assert_eq!(
            paths(&fresh),
            [".brokspace/databases/hot/gone", ".brokspace/databases/cold/chunks-00000001.arrow",]
        );

        let later = measure(&workspace, SystemTime::now() + TEMPORARY_AGE * 2);
        assert_eq!(
            paths(&later),
            [
                ".brokspace/databases/hot/gone",
                ".brokspace/databases/cold/chunks-00000001.arrow",
                ".brokspace/databases/cold/.files.json.99.tmp",
            ]
        );
        assert_eq!(later.leftover_bytes, 305 + 10 + 7);
        assert!(later.partitions.iter().any(|p| p.id == "gone" && p.orphaned));
    }

    #[test]
    #[cfg(unix)]
    fn links_are_not_followed() {
        let (dir, workspace) = workspace();
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "big", 10_000);
        fs::create_dir_all(dir.path().join(".brokspace/logs")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".brokspace/logs/elsewhere")).unwrap();
        assert!(report(&workspace).total_bytes < 10_000);
    }
}
