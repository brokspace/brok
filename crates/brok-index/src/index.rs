//! The workspace index: every partition, plus the indexing pipeline.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use brok_core::config::IndexConfig;
use brok_core::{BrokConfig, PartitionMap, Tier, Workspace, WorkspacePaths};
use brok_sdk::payload::IndexUpdated;
use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::chunk::{Chunk, ChunkOptions, chunk_file};
use crate::embed::{EmbedKind, Embedder};
use crate::partition_index::{EmbeddingInfo, FileRecord, PartitionIndex};

/// Files read, hashed and chunked per batch (bounds memory on big repos).
const FILE_BATCH: usize = 512;
/// Chunks embedded and appended per batch.
const CHUNK_BATCH: usize = 256;

/// How the index is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Search only; never modifies anything on disk.
    ReadOnly,
    /// Exclusive writer (holds `databases/.lock`).
    ReadWrite,
}

/// Indexing progress notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Walking the workspace.
    Scanning,
    /// Files that need (re)indexing in a partition.
    Partition {
        /// Partition id.
        id: String,
        /// Files to process.
        files: usize,
    },
    /// Files processed so far in the current partition.
    Files {
        /// Processed.
        done: usize,
        /// Total.
        total: usize,
    },
}

/// Outcome of an indexing run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexReport {
    /// Files seen on disk.
    pub files_scanned: usize,
    /// Files (re)chunked.
    pub files_indexed: usize,
    /// Files whose content did not change.
    pub files_unchanged: usize,
    /// Files removed from the index (deleted, excluded or binary).
    pub files_removed: usize,
    /// Chunks written.
    pub chunks: usize,
    /// Wall clock time.
    pub duration: Duration,
}

impl From<&IndexReport> for IndexUpdated {
    fn from(report: &IndexReport) -> Self {
        Self {
            files_indexed: report.files_indexed,
            files_removed: report.files_removed,
            chunks: report.chunks,
            duration_ms: u64::try_from(report.duration.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// Per-partition summary for `brok status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionStatus {
    /// Partition id.
    pub id: String,
    /// Workspace relative path (`""` for the root).
    pub path: String,
    /// Storage tier.
    pub tier: Tier,
    /// Store backend.
    pub backend: &'static str,
    /// Indexed files.
    pub files: usize,
    /// Stored chunks.
    pub chunks: usize,
    /// Whether chunks carry embeddings.
    pub semantic: bool,
}

/// The index of a whole workspace.
pub struct Index {
    pub(crate) root: PathBuf,
    pub(crate) config: BrokConfig,
    pub(crate) partitions: PartitionMap,
    pub(crate) parts: Vec<PartitionIndex>,
    pub(crate) embedder: Option<Arc<dyn Embedder>>,
    filter: FileFilter,
    _lock: Option<File>,
}

impl Index {
    /// Opens the index of `workspace`.
    ///
    /// In [`Access::ReadWrite`] mode the embedder must be present whenever
    /// embeddings are enabled, so that a missing model can never silently
    /// produce an index without vectors.
    pub async fn open(workspace: &Workspace, embedder: Option<Arc<dyn Embedder>>, access: Access) -> Result<Self> {
        let config = workspace.config().clone();
        let paths = workspace.paths();
        let writable = access == Access::ReadWrite;
        let lock = if writable { Some(acquire_write_lock(paths)?) } else { None };
        if writable && config.embeddings.enabled && embedder.is_none() {
            bail!("embeddings are enabled but no embedding model is loaded");
        }

        let embedding = embedder.as_ref().map(|e| EmbeddingInfo { model: e.model_id().to_owned(), dims: e.dims() });
        let partitions = PartitionMap::discover(workspace.root(), config.workspace.submodules);
        let mut parts = Vec::with_capacity(partitions.len());
        for partition in partitions.iter() {
            if !writable && !partition.database_dir(paths).exists() {
                continue; // never indexed: nothing to search, and read-only must not create it
            }
            let part =
                PartitionIndex::open(paths, partition.clone(), config.index.store, embedding.clone(), writable).await?;
            parts.push(part);
        }

        Ok(Self {
            root: workspace.root().to_path_buf(),
            filter: FileFilter::new(&config.index)?,
            config,
            partitions,
            parts,
            embedder,
            _lock: lock,
        })
    }

    /// Indexes the whole workspace incrementally (or from scratch with
    /// `rebuild`).
    pub async fn index_all(&mut self, rebuild: bool, progress: &(dyn Fn(Progress) + Sync)) -> Result<IndexReport> {
        let started = Instant::now();
        if rebuild {
            for part in &mut self.parts {
                part.reset(self.config.index.store).await?;
            }
        }
        progress(Progress::Scanning);
        let files = {
            let root = self.root.clone();
            tokio::task::spawn_blocking(move || walk(&root)).await??
        };
        let mut report = IndexReport { files_scanned: files.len(), ..IndexReport::default() };

        let mut by_partition: HashMap<String, Vec<FileMeta>> = HashMap::new();
        for file in files {
            if self.filter.allows(&file.rel) {
                by_partition.entry(self.partitions.resolve(&file.rel).id.clone()).or_default().push(file);
            }
        }

        for i in 0..self.parts.len() {
            let files = by_partition.remove(&self.parts[i].partition.id).unwrap_or_default();
            let present: HashSet<&str> = files.iter().map(|f| f.rel.as_str()).collect();
            let deleted: Vec<String> =
                self.parts[i].files().keys().filter(|path| !present.contains(path.as_str())).cloned().collect();
            let candidates: Vec<FileMeta> = files
                .into_iter()
                .filter(|f| {
                    self.parts[i].files().get(&f.rel).is_none_or(|r| r.size != f.size || r.modified != f.modified)
                })
                .collect();
            if candidates.is_empty() && deleted.is_empty() {
                continue;
            }
            progress(Progress::Partition { id: self.parts[i].partition.id.clone(), files: candidates.len() });
            self.process(i, candidates, deleted, progress, &mut report).await?;
        }
        report.duration = started.elapsed();
        Ok(report)
    }

    /// Applies a set of workspace relative changes (from `file:changed`).
    pub async fn apply_changes(&mut self, changed: &[String], deleted: &[String]) -> Result<IndexReport> {
        let started = Instant::now();
        let mut report = IndexReport::default();
        let mut work: HashMap<usize, (Vec<FileMeta>, Vec<String>)> = HashMap::new();

        let paths: BTreeSet<String> = changed.iter().chain(deleted).map(|p| normalize(p)).collect();
        for rel in paths {
            let Some(i) = self.part_index_for(&rel) else { continue };
            let entry = work.entry(i).or_default();
            let allowed = self.filter.allows(&rel) && !brok_core::fsutil::in_managed_dir(&self.root, &rel);
            match FileMeta::stat(&self.root, &rel).filter(|_| allowed) {
                Some(meta) => entry.0.push(meta),
                None if self.parts[i].files().contains_key(&rel) => entry.1.push(rel),
                None => {}
            }
        }
        for (i, (candidates, removed)) in work {
            report.files_scanned += candidates.len();
            self.process(i, candidates, removed, &|_| {}, &mut report).await?;
        }
        report.duration = started.elapsed();
        Ok(report)
    }

    /// Per-partition statistics.
    pub async fn status(&self) -> Result<Vec<PartitionStatus>> {
        let mut out = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            out.push(PartitionStatus {
                id: part.partition.id.clone(),
                path: part.partition.path.clone(),
                tier: part.partition.tier,
                backend: part.store.backend_name(),
                files: part.files().len(),
                chunks: part.store.count().await?,
                semantic: part.store.dims().is_some(),
            });
        }
        Ok(out)
    }

    /// The loaded embedder, if any.
    pub fn embedder(&self) -> Option<&Arc<dyn Embedder>> {
        self.embedder.as_ref()
    }

    fn part_index_for(&self, rel: &str) -> Option<usize> {
        let id = &self.partitions.resolve(rel).id;
        self.parts.iter().position(|p| &p.partition.id == id)
    }

    async fn process(
        &mut self,
        i: usize,
        candidates: Vec<FileMeta>,
        deleted: Vec<String>,
        progress: &(dyn Fn(Progress) + Sync),
        report: &mut IndexReport,
    ) -> Result<()> {
        let total = candidates.len();
        let options = ChunkOptions { max_lines: self.config.index.max_chunk_lines };
        let max_bytes = self.config.index.max_file_kb * 1024;

        // Removals first.
        if !deleted.is_empty() {
            self.parts[i].store.delete_paths(&deleted).await?;
            for path in &deleted {
                self.parts[i].bm25.remove_path(path);
                self.parts[i].forget(path);
            }
            report.files_removed += deleted.len();
        }

        let mut done = 0;
        let mut remaining = candidates;
        while !remaining.is_empty() {
            let batch: Vec<FileMeta> = remaining.drain(..remaining.len().min(FILE_BATCH)).collect();
            done += batch.len();
            let previous: HashMap<String, FileRecord> = batch
                .iter()
                .filter_map(|f| self.parts[i].files().get(&f.rel).map(|r| (f.rel.clone(), r.clone())))
                .collect();
            let outcomes = tokio::task::spawn_blocking(move || {
                batch
                    .into_par_iter()
                    .map(|file| read_and_chunk(file, &previous, max_bytes, options))
                    .collect::<Vec<_>>()
            })
            .await?;

            let mut stale = Vec::new();
            let mut chunks: Vec<Chunk> = Vec::new();
            let mut records = Vec::new();
            for outcome in outcomes {
                match outcome {
                    Outcome::Unchanged { rel, record } => {
                        report.files_unchanged += 1;
                        self.parts[i].record(rel, record);
                    }
                    Outcome::Skipped { rel } => {
                        if self.parts[i].files().contains_key(&rel) {
                            report.files_removed += 1;
                            self.parts[i].forget(&rel);
                            stale.push(rel);
                        }
                    }
                    Outcome::Chunked { rel, record, chunks: file_chunks } => {
                        report.files_indexed += 1;
                        stale.push(rel.clone());
                        records.push((rel, record));
                        chunks.extend(file_chunks);
                    }
                }
            }

            if !stale.is_empty() {
                self.parts[i].store.delete_paths(&stale).await?;
                for path in &stale {
                    self.parts[i].bm25.remove_path(path);
                }
            }
            for group in chunks.chunks(CHUNK_BATCH) {
                let vectors = self.embed_documents(group).await?;
                self.parts[i].store.append(group, vectors.as_deref()).await?;
                for chunk in group {
                    self.parts[i].bm25.add(&chunk.id, &chunk.path, &chunk.embedding_text());
                }
            }
            report.chunks += chunks.len();
            for (rel, record) in records {
                self.parts[i].record(rel, record);
            }
            progress(Progress::Files { done, total });
        }

        self.parts[i].save().await
    }

    async fn embed_documents(&self, chunks: &[Chunk]) -> Result<Option<Vec<Vec<f32>>>> {
        let Some(embedder) = self.embedder.clone() else { return Ok(None) };
        let texts: Vec<String> = chunks.iter().map(Chunk::embedding_text).collect();
        let vectors = tokio::task::spawn_blocking(move || embedder.embed(&texts, EmbedKind::Document)).await??;
        Ok(Some(vectors))
    }
}

fn acquire_write_lock(paths: &WorkspacePaths) -> Result<File> {
    std::fs::create_dir_all(&paths.databases)?;
    let path = paths.databases.join(".lock");
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => {
            bail!("another brok process is writing the index (`brok run` indexes changes automatically while it runs)")
        }
        Err(TryLockError::Error(e)) => Err(e).with_context(|| format!("locking {}", path.display())),
    }
}

/// Include/exclude rules from `brok.yml`.
struct FileFilter {
    include: GlobSet,
    exclude: GlobSet,
}

impl FileFilter {
    fn new(config: &IndexConfig) -> Result<Self> {
        let build = |patterns: &[String]| -> Result<GlobSet> {
            let mut builder = GlobSetBuilder::new();
            for pattern in patterns {
                builder.add(Glob::new(pattern).with_context(|| format!("invalid glob `{pattern}` in brok.yml"))?);
            }
            Ok(builder.build()?)
        };
        Ok(Self { include: build(&config.include)?, exclude: build(&config.exclude)? })
    }

    fn allows(&self, rel: &str) -> bool {
        self.include.is_match(rel) && !self.exclude.is_match(rel)
    }
}

#[derive(Debug, Clone)]
struct FileMeta {
    rel: String,
    abs: PathBuf,
    size: u64,
    modified: u64,
}

impl FileMeta {
    fn from_metadata(rel: String, abs: PathBuf, metadata: &std::fs::Metadata) -> Self {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        Self { rel, abs, size: metadata.len(), modified }
    }

    fn stat(root: &Path, rel: &str) -> Option<Self> {
        let abs = root.join(rel);
        let metadata = std::fs::metadata(&abs).ok().filter(std::fs::Metadata::is_file)?;
        Some(Self::from_metadata(rel.to_owned(), abs, &metadata))
    }
}

/// Lists indexable files, honoring `.gitignore` (including nested ones).
fn walk(root: &Path) -> Result<Vec<FileMeta>> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| {
            let skip_name = matches!(entry.file_name().to_str(), Some(".git" | ".brokspace"));
            // Folders brok generated for other tools (exported skills) are not workspace knowledge.
            let generated = entry.file_type().is_some_and(|t| t.is_dir())
                && entry.path().join(brok_core::fsutil::MANAGED_MARKER).is_file();
            !skip_name && !generated
        })
        .build();
    let mut files = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::debug!(error = %e, "skipping unreadable path");
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel) = brok_core::fsutil::to_workspace_relative(root, entry.path()) else { continue };
        if let Ok(metadata) = entry.metadata() {
            files.push(FileMeta::from_metadata(rel, entry.path().to_path_buf(), &metadata));
        }
    }
    Ok(files)
}

enum Outcome {
    Unchanged { rel: String, record: FileRecord },
    Skipped { rel: String },
    Chunked { rel: String, record: FileRecord, chunks: Vec<Chunk> },
}

fn read_and_chunk(
    file: FileMeta,
    previous: &HashMap<String, FileRecord>,
    max_bytes: u64,
    options: ChunkOptions,
) -> Outcome {
    if file.size > max_bytes {
        return Outcome::Skipped { rel: file.rel };
    }
    let Ok(bytes) = std::fs::read(&file.abs) else { return Outcome::Skipped { rel: file.rel } };
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Outcome::Skipped { rel: file.rel }; // binary
    }
    let hash = hex::encode(Sha256::digest(&bytes));
    let record = |chunks: u32| FileRecord { hash: hash.clone(), size: file.size, modified: file.modified, chunks };
    if let Some(old) = previous.get(&file.rel).filter(|old| old.hash == hash) {
        // Touched but identical: only refresh the stat data.
        return Outcome::Unchanged { record: record(old.chunks), rel: file.rel };
    }
    let Ok(text) = String::from_utf8(bytes) else { return Outcome::Skipped { rel: file.rel } };
    let chunks = chunk_file(&file.rel, &text, &hash, options);
    Outcome::Chunked { record: record(u32::try_from(chunks.len()).unwrap_or(u32::MAX)), rel: file.rel, chunks }
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}
