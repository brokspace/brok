//! The workspace index: every partition, plus the indexing pipeline.

use std::collections::{BTreeSet, HashMap};
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
use crate::links::{LinkStatus, LinkedIndex};
use crate::memory::MemorySearch;
use crate::os;
use crate::partition_index::{EmbeddingInfo, FileRecord, MANIFEST_FILE, PartitionIndex};

/// Files read, hashed and chunked per batch (bounds memory on big repos).
const FILE_BATCH: usize = 512;
/// Chunks embedded and appended per batch.
const CHUNK_BATCH: usize = 256;
/// How often the vector pass saves what it embedded (ADR 0018).
const CHECKPOINT: Duration = Duration::from_secs(60);
/// A save may take at most this fraction (1 / n) of the time between saves.
const SAVE_SHARE: u32 = 50;
/// Fewest and most chunks embedded between two saves.
const MIN_JOB: usize = 64;
const MAX_JOB: usize = 4096;
/// Replaced data younger than this is kept: a reader may still be using it.
const KEEP_REPLACED: Duration = Duration::from_secs(10 * 60);

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
    /// Every change is searchable by keywords; `waiting` chunks still have to
    /// be embedded.
    TextReady {
        /// Chunks without a vector yet.
        waiting: usize,
    },
    /// Chunks embedded and saved so far.
    Vectors {
        /// Saved.
        done: usize,
        /// Waiting when this pass started.
        total: usize,
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
    /// Indexed chunks.
    pub chunks: usize,
    /// Chunks that have their vector (0 without semantic search).
    pub embedded: usize,
    /// Chunks still waiting for their vector.
    pub waiting: usize,
    /// Whether searches use the vectors: enough chunks have one.
    pub semantic: bool,
}

/// Embeds some waiting files. It borrows nothing from the index, so searches
/// go on while it runs; [`Index::save_vectors`] applies the result.
pub struct VectorJob {
    partition: usize,
    /// Path and the content hash the index has for it.
    files: Vec<(String, String)>,
    root: PathBuf,
    embedder: Arc<dyn Embedder>,
    options: ChunkOptions,
    max_bytes: u64,
}

/// What a [`VectorJob`] computed.
pub struct VectorBatch {
    partition: usize,
    files: Vec<EmbeddedFile>,
}

/// A waiting file as it is on disk now: path, expected hash and content.
type ReadFile = (String, String, Option<(FileRecord, Vec<Chunk>)>);

struct EmbeddedFile {
    rel: String,
    /// The hash the index had when the job was made.
    expected: String,
    /// `None` when the file is gone or no longer indexable.
    content: Option<(FileRecord, Vec<Chunk>, Vec<Vec<f32>>)>,
}

impl VectorJob {
    /// Reads, chunks and embeds the files.
    pub async fn run(self) -> Result<VectorBatch> {
        let VectorJob { partition, files, root, embedder, options, max_bytes } = self;
        tokio::task::spawn_blocking(move || {
            let read: Vec<ReadFile> = files
                .into_iter()
                .map(|(rel, expected)| {
                    let outcome = FileMeta::stat(&root, &rel)
                        .map(|file| read_and_chunk(file, &HashMap::new(), max_bytes, options));
                    let content = match outcome {
                        Some(Outcome::Chunked { record, chunks, .. }) => Some((record, chunks)),
                        _ => None,
                    };
                    (rel, expected, content)
                })
                .collect();
            // Embedded together, whatever file they come from: batches stay full.
            let texts: Vec<String> = read
                .iter()
                .filter_map(|(_, _, content)| content.as_ref())
                .flat_map(|(_, chunks)| chunks.iter().map(Chunk::embedding_text))
                .collect();
            let mut vectors = Vec::with_capacity(texts.len());
            for group in texts.chunks(CHUNK_BATCH) {
                vectors.extend(embedder.embed(group, EmbedKind::Document)?);
            }
            anyhow::ensure!(
                vectors.len() == texts.len(),
                "the model returned {} vectors for {} texts",
                vectors.len(),
                texts.len()
            );
            let mut vectors = vectors.into_iter();
            let out = read
                .into_iter()
                .map(|(rel, expected, content)| {
                    let content = content.map(|(record, chunks)| {
                        let own = vectors.by_ref().take(chunks.len()).collect();
                        (record, chunks, own)
                    });
                    EmbeddedFile { rel, expected, content }
                })
                .collect();
            Ok(VectorBatch { partition, files: out })
        })
        .await?
    }
}

/// The index of a whole workspace.
pub struct Index {
    pub(crate) root: PathBuf,
    pub(crate) config: BrokConfig,
    pub(crate) partitions: PartitionMap,
    pub(crate) parts: Vec<PartitionIndex>,
    pub(crate) embedder: Option<Arc<dyn Embedder>>,
    /// Linked workspaces, opened on the first search.
    pub(crate) links: tokio::sync::Mutex<Vec<LinkedIndex>>,
    pub(crate) memory: MemorySearch,
    /// Local metrics of this workspace (`brok stats`).
    pub(crate) metrics: brok_core::metrics::Recorder,
    filter: FileFilter,
    checkpoint: Duration,
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

        let links = LinkedIndex::all(workspace);
        let shown: Vec<_> = links.iter().map(|l| (l.link.clone(), l.prefix.clone())).collect();
        let memory = MemorySearch::new(workspace, &shown);
        let metrics = brok_core::metrics::Recorder::new(paths, &config.metrics);
        Ok(Self {
            root: workspace.root().to_path_buf(),
            filter: FileFilter::new(&config.index)?,
            config,
            partitions,
            parts,
            embedder,
            links: tokio::sync::Mutex::new(links),
            memory,
            metrics,
            checkpoint: CHECKPOINT,
            _lock: lock,
        })
    }

    /// Indexes the whole workspace incrementally (or from scratch with
    /// `rebuild`): first the text of every change, then the vectors, saved
    /// as they are computed. Interrupted, it resumes where it stopped.
    pub async fn index_all(&mut self, rebuild: bool, progress: &(dyn Fn(Progress) + Sync)) -> Result<IndexReport> {
        let started = Instant::now();
        let mut report = self.index_text(rebuild, progress).await?;
        self.fill_vectors(progress).await?;
        report.duration = started.elapsed();
        Ok(report)
    }

    /// The first pass of [`Self::index_all`]: afterwards every change is
    /// found by keywords, and chunks may wait for their vectors.
    pub async fn index_text(&mut self, rebuild: bool, progress: &(dyn Fn(Progress) + Sync)) -> Result<IndexReport> {
        let started = Instant::now();
        if rebuild {
            for part in &mut self.parts {
                part.reset(self.config.index.store).await?;
            }
        }
        progress(Progress::Scanning);
        let (files_scanned, work) = self.scan().await?;
        let mut report = IndexReport { files_scanned, ..IndexReport::default() };

        for (i, (candidates, deleted)) in work.into_iter().enumerate() {
            if candidates.is_empty() && deleted.is_empty() {
                continue;
            }
            progress(Progress::Partition { id: self.parts[i].partition.id.clone(), files: candidates.len() });
            self.process(i, candidates, deleted, progress, &mut report).await?;
        }
        if self.waiting_chunks() > 0 {
            // Embedding runs next to this process' largest buffers.
            os::release_freed_memory();
        }
        report.duration = started.elapsed();
        self.record_index("full", &report);
        Ok(report)
    }

    /// Walks the workspace and returns the number of files seen and, for each
    /// partition, the files to (re)index and the ones that disappeared. Files
    /// arrive in batches and unchanged ones are dropped at once, so memory
    /// grows with what changed rather than with the size of the workspace.
    async fn scan(&self) -> Result<(usize, Vec<PartitionWork>)> {
        let (batches, mut walked) = tokio::sync::mpsc::channel(SCAN_BATCHES);
        let root = self.root.clone();
        let walker = tokio::task::spawn_blocking(move || walk(&root, &batches));

        let positions: HashMap<&str, usize> =
            self.parts.iter().enumerate().map(|(i, part)| (part.partition.id.as_str(), i)).collect();
        // Indexed files not seen yet; whatever is left at the end was deleted.
        let mut unseen: Vec<BTreeSet<&str>> =
            self.parts.iter().map(|part| part.files().keys().map(String::as_str).collect()).collect();
        let mut work: Vec<PartitionWork> = self.parts.iter().map(|_| PartitionWork::default()).collect();
        let mut scanned = 0;
        while let Some(batch) = walked.recv().await {
            scanned += batch.len();
            for file in batch {
                if !self.filter.allows(&file.rel) {
                    continue;
                }
                let Some(&i) = positions.get(self.partitions.resolve(&file.rel).id.as_str()) else { continue };
                unseen[i].remove(file.rel.as_str());
                let record = self.parts[i].files().get(&file.rel);
                if record.is_none_or(|r| r.size != file.size || r.modified != file.modified) {
                    work[i].0.push(file);
                }
            }
        }
        // A walk that failed halfway must not read as files being deleted.
        walker.await??;
        for (work, unseen) in work.iter_mut().zip(unseen) {
            work.1 = unseen.into_iter().map(str::to_owned).collect();
        }
        Ok((scanned, work))
    }

    /// Chunks waiting for a vector, in every partition.
    pub fn waiting_chunks(&self) -> usize {
        if self.embedder.is_none() {
            return 0;
        }
        self.parts.iter().map(|part| part.chunk_counts().1).sum()
    }

    /// The next files to embed, about `max_chunks` chunks of one partition;
    /// `None` when nothing waits.
    pub fn vector_job(&self, max_chunks: usize) -> Option<VectorJob> {
        let embedder = self.embedder.clone()?;
        for (partition, part) in self.parts.iter().enumerate() {
            if !part.defers_vectors() {
                continue;
            }
            let mut chunks = 0;
            let mut files = Vec::new();
            for (rel, record) in part.files().iter().filter(|(_, record)| record.pending) {
                chunks += record.chunks as usize;
                files.push((rel.clone(), record.hash.clone()));
                if chunks >= max_chunks {
                    break;
                }
            }
            if !files.is_empty() {
                return Some(VectorJob {
                    partition,
                    files,
                    root: self.root.clone(),
                    embedder,
                    options: ChunkOptions { max_lines: self.config.index.max_chunk_lines },
                    max_bytes: self.config.index.max_file_kb * 1024,
                });
            }
        }
        None
    }

    /// Stores what a job computed and returns the chunks that got a vector.
    /// A file indexed again since the job was made is left for a later one.
    pub async fn save_vectors(&mut self, batch: VectorBatch) -> Result<usize> {
        let part = &mut self.parts[batch.partition];
        let mut replaced = Vec::new();
        let mut apply = Vec::new();
        for file in batch.files {
            let current = part.files().get(&file.rel).is_some_and(|r| r.pending && r.hash == file.expected);
            if current {
                replaced.push(file.rel.clone());
                apply.push(file);
            }
        }
        if apply.is_empty() {
            return Ok(0);
        }
        part.store.delete_paths(&replaced).await?;
        let mut saved = 0;
        for file in apply {
            let Some((record, chunks, vectors)) = file.content else {
                part.bm25.remove_path(&file.rel);
                part.forget(&file.rel);
                continue;
            };
            // Edited since the text pass: its keywords changed too.
            if record.hash != file.expected {
                part.bm25.remove_path(&file.rel);
                for chunk in &chunks {
                    part.bm25.add(&chunk.id, &chunk.path, &chunk.embedding_text());
                }
            }
            for (group, vectors) in chunks.chunks(CHUNK_BATCH).zip(vectors.chunks(CHUNK_BATCH)) {
                part.store.append(group, Some(vectors)).await?;
            }
            saved += chunks.len();
            part.record(file.rel, record);
        }
        part.save().await?;
        os::release_freed_memory();
        Ok(saved)
    }

    /// The second pass of [`Self::index_all`]: embeds everything waiting,
    /// saving about once a minute.
    pub async fn fill_vectors(&mut self, progress: &(dyn Fn(Progress) + Sync)) -> Result<()> {
        let total = self.waiting_chunks();
        if total == 0 {
            return Ok(());
        }
        progress(Progress::TextReady { waiting: total });
        let mut pace = Pace::new(self.checkpoint);
        let mut done = 0;
        while let Some(job) = self.vector_job(pace.chunks) {
            let started = Instant::now();
            let batch = job.run().await?;
            let embedding = started.elapsed();
            let saving = Instant::now();
            let saved = self.save_vectors(batch).await?;
            pace.record(saved, embedding, saving.elapsed());
            done += saved;
            progress(Progress::Vectors { done: done.min(total), total });
        }
        self.prune_replaced().await;
        Ok(())
    }

    /// Frees the space of rows replaced a while ago. Failing costs disk only.
    pub async fn prune_replaced(&mut self) {
        for part in &mut self.parts {
            if let Err(e) = part.store.prune(KEEP_REPLACED).await {
                tracing::warn!(partition = %part.partition.id, error = %e, "cannot remove replaced index data");
            }
        }
    }

    pub(crate) fn pace(&self) -> Pace {
        Pace::new(self.checkpoint)
    }

    /// When any partition was last saved to disk: a reader holding an older
    /// snapshot opens the index again.
    pub fn saved_at(paths: &WorkspacePaths) -> Option<std::time::SystemTime> {
        let manifest = |dir: PathBuf| std::fs::metadata(dir.join(MANIFEST_FILE)).and_then(|m| m.modified()).ok();
        let nested = std::fs::read_dir(paths.databases.join("hot")).into_iter().flatten().flatten();
        std::iter::once(paths.databases.join("cold")).chain(nested.map(|e| e.path())).filter_map(manifest).max()
    }

    /// Saves vectors this often instead of about once a minute (tests).
    #[doc(hidden)]
    pub fn set_checkpoint(&mut self, interval: Duration) {
        self.checkpoint = interval;
    }

    /// Applies a set of workspace relative changes (from `file:changed`).
    pub async fn apply_changes(&mut self, changed: &[String], deleted: &[String]) -> Result<IndexReport> {
        let mut report = self.apply_changes_text(changed, deleted).await?;
        let started = Instant::now();
        self.fill_vectors(&|_| {}).await?;
        report.duration += started.elapsed();
        Ok(report)
    }

    /// [`Self::apply_changes`] without computing the vectors.
    pub async fn apply_changes_text(&mut self, changed: &[String], deleted: &[String]) -> Result<IndexReport> {
        let started = Instant::now();
        let mut report = IndexReport::default();
        let mut work: HashMap<usize, (Vec<FileMeta>, Vec<String>)> = HashMap::new();

        let paths: BTreeSet<String> = changed.iter().chain(deleted).map(|p| normalize(p)).collect();
        for rel in paths {
            let Some(i) = self.part_index_for(&rel) else { continue };
            let entry = work.entry(i).or_default();
            let allowed = self.filter.allows(&rel)
                && !brok_core::fsutil::in_managed_dir(&self.root, &rel)
                && !is_shared_memory(&rel);
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
        if report.files_indexed + report.files_removed > 0 {
            self.record_index("changes", &report);
        }
        Ok(report)
    }

    fn record_index(&self, mode: &str, report: &IndexReport) {
        self.metrics.record(
            "index",
            Some(report.duration),
            serde_json::json!({
                "mode": mode,
                "files_scanned": report.files_scanned,
                "files_indexed": report.files_indexed,
                "files_removed": report.files_removed,
                "chunks": report.chunks,
            }),
        );
    }

    /// Per-partition statistics.
    pub async fn status(&self) -> Result<Vec<PartitionStatus>> {
        let mut out = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            // The manifest is what is indexed; a store may hold rows of an
            // interrupted run until the next one replaces them.
            let (chunks, waiting) = part.chunk_counts();
            out.push(PartitionStatus {
                id: part.partition.id.clone(),
                path: part.partition.path.clone(),
                tier: part.partition.tier,
                backend: part.store.backend_name(),
                files: part.files().len(),
                chunks,
                embedded: if part.store.dims().is_some() { chunks - waiting } else { 0 },
                waiting: if part.store.dims().is_some() { waiting } else { 0 },
                semantic: part.searches_by_meaning(),
            });
        }
        Ok(out)
    }

    /// Linked workspaces and whether they can be searched.
    pub async fn link_status(&self) -> Vec<LinkStatus> {
        let mut links = self.links.lock().await;
        let mut out = Vec::with_capacity(links.len());
        for link in links.iter_mut() {
            out.push(link.status(&self.config).await);
        }
        out
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
            // A store that allows it gets the text now and the vectors in
            // the second pass.
            let defer = self.embedder.is_some() && self.parts[i].defers_vectors();
            for group in chunks.chunks(CHUNK_BATCH) {
                let vectors = if defer { None } else { self.embed_documents(group).await? };
                self.parts[i].store.append(group, vectors.as_deref()).await?;
                for chunk in group {
                    self.parts[i].bm25.add(&chunk.id, &chunk.path, &chunk.embedding_text());
                }
            }
            report.chunks += chunks.len();
            for (rel, mut record) in records {
                record.pending = defer && record.chunks > 0;
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

/// How many chunks to embed before the next save: about `checkpoint` worth
/// of work, longer when saving is slow (a large Arrow store).
pub(crate) struct Pace {
    checkpoint: Duration,
    pub(crate) chunks: usize,
}

impl Pace {
    pub(crate) fn new(checkpoint: Duration) -> Self {
        Self { checkpoint, chunks: MIN_JOB }
    }

    pub(crate) fn record(&mut self, saved: usize, embedding: Duration, saving: Duration) {
        // A zero checkpoint (tests) saves as often as it can.
        let spaced = if self.checkpoint.is_zero() { Duration::ZERO } else { saving * SAVE_SHARE };
        let interval = self.checkpoint.max(spaced);
        let per_second = saved as f64 / embedding.as_secs_f64().max(0.001);
        // Grows at most four times at once: the first measure is rough.
        let wanted = (per_second * interval.as_secs_f64()) as usize;
        self.chunks = wanted.clamp(MIN_JOB, MAX_JOB.min(self.chunks * 4));
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

/// A partition's files to (re)index, and its files that disappeared.
type PartitionWork = (Vec<FileMeta>, Vec<String>);

/// Files per batch sent by the walker, and batches that may wait.
const SCAN_BATCH: usize = 512;
const SCAN_BATCHES: usize = 4;

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

/// Sends the indexable files to `batches`, honoring `.gitignore` (including
/// nested ones). Stops early if the receiver is gone.
fn walk(root: &Path, batches: &tokio::sync::mpsc::Sender<Vec<FileMeta>>) -> Result<()> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| {
            // Shared memories are searched as memories, not as code.
            let skip_name = matches!(
                entry.file_name().to_str(),
                Some(".git" | ".brokspace" | brok_core::workspace::SHARED_MEMORY_DIR)
            );
            // Folders brok generated for other tools (exported skills) are not workspace knowledge.
            let generated = entry.file_type().is_some_and(|t| t.is_dir())
                && entry.path().join(brok_core::fsutil::MANAGED_MARKER).is_file();
            !skip_name && !generated
        })
        .build();
    let mut files = Vec::with_capacity(SCAN_BATCH);
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
        if files.len() == SCAN_BATCH {
            let batch = std::mem::replace(&mut files, Vec::with_capacity(SCAN_BATCH));
            if batches.blocking_send(batch).is_err() {
                return Ok(());
            }
        }
    }
    let _ = batches.blocking_send(files);
    Ok(())
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
    let record = |chunks: u32| FileRecord {
        hash: hash.clone(),
        size: file.size,
        modified: file.modified,
        chunks,
        pending: false,
    };
    if let Some(old) = previous.get(&file.rel).filter(|old| old.hash == hash) {
        // Touched but identical: only refresh the stat data.
        return Outcome::Unchanged { record: FileRecord { pending: old.pending, ..record(old.chunks) }, rel: file.rel };
    }
    let Ok(text) = String::from_utf8(bytes) else { return Outcome::Skipped { rel: file.rel } };
    let chunks = chunk_file(&file.rel, &text, &hash, options);
    Outcome::Chunked { record: record(u32::try_from(chunks.len()).unwrap_or(u32::MAX)), rel: file.rel, chunks }
}

fn is_shared_memory(rel: &str) -> bool {
    rel.split('/').next() == Some(brok_core::workspace::SHARED_MEMORY_DIR)
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pace_follows_the_machine_within_limits() {
        let second = Duration::from_secs(1);
        let mut pace = Pace::new(Duration::from_secs(60));
        assert_eq!(pace.chunks, MIN_JOB);
        pace.record(64, second, Duration::ZERO);
        assert_eq!(pace.chunks, 256, "grows gradually");
        pace.record(256, second * 4, Duration::ZERO);
        pace.record(1024, second * 16, Duration::ZERO);
        assert_eq!(pace.chunks, MAX_JOB.min(64 * 60));
        pace.record(100, second * 1000, Duration::ZERO);
        assert_eq!(pace.chunks, MIN_JOB, "a slow machine still saves whole files");
        let mut paused = Pace::new(Duration::ZERO);
        paused.record(1000, second, Duration::ZERO);
        assert_eq!(paused.chunks, MIN_JOB);
        pace.record(64, second, second * 10);
        assert_eq!(pace.chunks, 256, "slow saves are spaced out");
    }
}
