//! The on-disk state of one partition: chunk store, BM25 index and the file
//! manifest that makes indexing incremental.
//!
//! ```text
//! databases/cold/            (or databases/hot/<id>/)
//! ├── files.json   manifest: path → hash, size, mtime, chunk count
//! ├── bm25.bin     BM25 index (postcard)
//! └── ...          chunk store (LanceDB table or Arrow generations)
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brok_core::config::StoreBackend;
use brok_core::{Partition, WorkspacePaths};
use serde::{Deserialize, Serialize};

use crate::bm25::Bm25Index;
use crate::schema::Precision;
use crate::store::{ChunkStore, resolve_backend};

pub(crate) const MANIFEST_FILE: &str = "files.json";
pub(crate) const BM25_FILE: &str = "bm25.bin";
const MANIFEST_VERSION: u32 = 1;
/// Manifest of a partition with `Float16` vectors. brok 0.10 and earlier do
/// not know it, so they rebuild the partition instead of finding no vectors.
const HALF_VECTORS_VERSION: u32 = 2;
/// Manifest of a partition whose rows may wait for their vectors (ADR 0018).
/// brok 0.11 and earlier rebuild it.
const PENDING_VECTORS_VERSION: u32 = 3;

/// Embedding settings an index was built with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingInfo {
    /// Model id.
    pub model: String,
    /// Vector dimensions.
    pub dims: usize,
}

/// What we know about an indexed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// SHA-256 of the content.
    pub hash: String,
    /// Size in bytes when indexed.
    pub size: u64,
    /// Modification time (ns since epoch) when indexed.
    pub modified: u64,
    /// Number of chunks produced.
    pub chunks: u32,
    /// The chunks are stored without vectors, which are still to be computed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    backend: String,
    embedding: Option<EmbeddingInfo>,
    files: BTreeMap<String, FileRecord>,
}

impl Manifest {
    fn empty(backend: &str, embedding: Option<EmbeddingInfo>) -> Self {
        Self { version: MANIFEST_VERSION, backend: backend.to_owned(), embedding, files: BTreeMap::new() }
    }
}

/// One partition's index.
pub struct PartitionIndex {
    /// The partition.
    pub partition: Partition,
    dir: PathBuf,
    pub(crate) store: ChunkStore,
    pub(crate) bm25: Bm25Index,
    manifest: Manifest,
}

impl PartitionIndex {
    /// Opens the partition. In write mode, an index built with other
    /// settings (backend, embedding model or dimensions) is discarded so it
    /// can be rebuilt consistently.
    pub async fn open(
        paths: &WorkspacePaths,
        partition: Partition,
        backend: StoreBackend,
        embedding: Option<EmbeddingInfo>,
        writable: bool,
    ) -> Result<Self> {
        let dir = partition.database_dir(paths);
        let backend_name = match resolve_backend(backend)? {
            StoreBackend::Lancedb => "lancedb",
            _ => "arrow",
        };
        let stored: Option<Manifest> =
            std::fs::read(dir.join(MANIFEST_FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let expected = Manifest::empty(backend_name, embedding.clone());

        let manifest = match stored {
            Some(m)
                if matches!(m.version, MANIFEST_VERSION | HALF_VECTORS_VERSION | PENDING_VECTORS_VERSION)
                    && m.backend == expected.backend
                    && m.embedding == expected.embedding =>
            {
                m
            }
            Some(_) if writable => {
                tracing::info!(partition = %partition.id, "index settings changed; rebuilding the partition");
                wipe(&dir)?;
                expected
            }
            // Read-only: keep whatever is there and search what is compatible.
            Some(m) => m,
            None => expected,
        };

        let store = ChunkStore::open(&dir, backend, embedding.as_ref().map(|e| e.dims)).await?;
        let bm25 = Bm25Index::load(&dir.join(BM25_FILE));
        Ok(Self { partition, dir, store, bm25, manifest })
    }

    /// Embedding model and dimensions the stored vectors were made with.
    pub fn embedding(&self) -> Option<&EmbeddingInfo> {
        self.manifest.embedding.as_ref()
    }

    /// Recorded state of every indexed file.
    pub fn files(&self) -> &BTreeMap<String, FileRecord> {
        &self.manifest.files
    }

    pub(crate) fn record(&mut self, path: String, record: FileRecord) {
        self.manifest.files.insert(path, record);
    }

    pub(crate) fn forget(&mut self, path: &str) {
        self.manifest.files.remove(path);
    }

    /// Whether chunks can be stored first and get their vectors later.
    pub fn defers_vectors(&self) -> bool {
        self.store.dims().is_some() && self.store.format().optional
    }

    /// Chunks recorded in the manifest, and how many of them still wait for a
    /// vector.
    pub fn chunk_counts(&self) -> (usize, usize) {
        self.manifest.files.values().fold((0, 0), |(all, waiting), file| {
            let chunks = file.chunks as usize;
            (all + chunks, waiting + if file.pending { chunks } else { 0 })
        })
    }

    /// Whether searches use this partition's vectors: at least 95% of its
    /// chunks have one. Ranking a partly embedded partition by meaning would
    /// favour whatever was embedded first.
    pub fn searches_by_meaning(&self) -> bool {
        let (chunks, waiting) = self.chunk_counts();
        self.store.dims().is_some() && waiting.saturating_mul(20) <= chunks
    }

    /// Commits the store and persists BM25 and the manifest.
    pub async fn save(&mut self) -> Result<()> {
        self.store.commit().await?;
        let format = self.store.format();
        self.manifest.version = match self.store.dims() {
            Some(_) if format.optional => PENDING_VECTORS_VERSION,
            Some(_) if format.precision == Precision::Half => HALF_VECTORS_VERSION,
            _ => MANIFEST_VERSION,
        };
        self.bm25.save(&self.dir.join(BM25_FILE))?;
        let json = serde_json::to_vec_pretty(&self.manifest)?;
        brok_core::fsutil::write_atomic(&self.dir.join(MANIFEST_FILE), &json)?;
        Ok(())
    }

    /// Deletes everything and reopens an empty partition.
    pub async fn reset(&mut self, backend: StoreBackend) -> Result<()> {
        let embedding = self.manifest.embedding.clone();
        wipe(&self.dir)?;
        self.store = ChunkStore::open(&self.dir, backend, embedding.as_ref().map(|e| e.dims)).await?;
        self.bm25 = Bm25Index::default();
        self.manifest = Manifest::empty(self.store.backend_name(), embedding);
        Ok(())
    }
}

fn wipe(dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use brok_core::{PartitionMap, Workspace};

    use super::*;
    use crate::schema::VectorFormat;
    use crate::schema::tests::sample;

    #[tokio::test]
    async fn an_index_with_single_precision_is_kept_until_rebuilt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".brokspace")).unwrap();
        std::fs::write(dir.path().join(".brokspace/brok.yml"), "version: 1\n").unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        let partition = PartitionMap::discover(dir.path(), false).root().clone();
        let embedding = Some(EmbeddingInfo { model: "test".into(), dims: 2 });
        let open =
            || PartitionIndex::open(workspace.paths(), partition.clone(), StoreBackend::Arrow, embedding.clone(), true);
        let version = || -> u32 {
            let bytes = std::fs::read(partition.database_dir(workspace.paths()).join(MANIFEST_FILE)).unwrap();
            serde_json::from_slice::<Manifest>(&bytes).unwrap().version
        };

        // What brok 0.10 wrote: version 1 and required `Float32` vectors.
        let mut old = open().await.unwrap();
        // Irrefutable in builds without LanceDB.
        #[allow(irrefutable_let_patterns)]
        let ChunkStore::Arrow(store) = &mut old.store else { unreachable!("the arrow backend") };
        store.set_format(VectorFormat { precision: Precision::Single, optional: false });
        old.store.append(&[sample("a1", "a.rs")], Some(&[vec![1.0, 0.0]])).await.unwrap();
        old.record("a.rs".into(), FileRecord { hash: "h".into(), size: 1, modified: 1, chunks: 1, pending: false });
        old.save().await.unwrap();
        assert_eq!(version(), 1);

        let mut kept = open().await.unwrap();
        assert_eq!((kept.files().len(), kept.store.format().precision), (1, Precision::Single), "not rebuilt");
        assert!(!kept.defers_vectors(), "its table requires them");
        kept.save().await.unwrap();
        assert_eq!(version(), 1);

        kept.reset(StoreBackend::Arrow).await.unwrap();
        kept.store.append(&[sample("a1", "a.rs")], Some(&[vec![1.0, 0.0]])).await.unwrap();
        kept.record("a.rs".into(), FileRecord { hash: "h".into(), size: 1, modified: 1, chunks: 1, pending: false });
        kept.save().await.unwrap();
        assert_eq!(version(), 3);
        let reopened = open().await.unwrap();
        assert_eq!((reopened.files().len(), reopened.store.format()), (1, VectorFormat::default()));
    }
}
