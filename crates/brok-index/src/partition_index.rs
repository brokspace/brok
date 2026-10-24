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
use crate::store::{ChunkStore, resolve_backend};

const MANIFEST_FILE: &str = "files.json";
const BM25_FILE: &str = "bm25.bin";
const MANIFEST_VERSION: u32 = 1;

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
                if m.version == expected.version
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

    /// Commits the store and persists BM25 and the manifest.
    pub async fn save(&mut self) -> Result<()> {
        self.store.commit().await?;
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
