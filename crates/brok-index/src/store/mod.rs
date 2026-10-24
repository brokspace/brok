//! Chunk storage backends.
//!
//! - [`lance`] (feature `lancedb`): LanceDB tables with ANN vector indexes;
//!   the default, scales to very large workspaces.
//! - [`arrow`]: immutable, memory-mapped Arrow IPC generations with exact
//!   (flat) vector search; zero native dependencies.
//!
//! Both share the schema in [`crate::schema`]. Writes are staged with
//! [`ChunkStore::delete_paths`] / [`ChunkStore::append`] and published by
//! [`ChunkStore::commit`].

pub mod arrow;
#[cfg(feature = "lancedb")]
pub mod lance;

use std::path::Path;

use anyhow::{Result, bail};
use brok_core::config::StoreBackend;

use crate::chunk::Chunk;

/// A chunk store, dispatched statically over the compiled backends.
pub enum ChunkStore {
    /// Arrow IPC files.
    Arrow(arrow::ArrowStore),
    /// LanceDB table.
    #[cfg(feature = "lancedb")]
    Lance(lance::LanceStore),
}

/// Resolves `auto` to a concrete backend available in this build.
pub fn resolve_backend(requested: StoreBackend) -> Result<StoreBackend> {
    match requested {
        StoreBackend::Auto if cfg!(feature = "lancedb") => Ok(StoreBackend::Lancedb),
        StoreBackend::Auto | StoreBackend::Arrow => Ok(StoreBackend::Arrow),
        StoreBackend::Lancedb if cfg!(feature = "lancedb") => Ok(StoreBackend::Lancedb),
        StoreBackend::Lancedb => {
            bail!("index.store is `lancedb` but brok was built without the `lancedb` feature; use `auto` or `arrow`")
        }
    }
}

impl ChunkStore {
    /// Opens (or creates) the store in `dir`.
    pub async fn open(dir: &Path, backend: StoreBackend, dims: Option<usize>) -> Result<Self> {
        match resolve_backend(backend)? {
            #[cfg(feature = "lancedb")]
            StoreBackend::Lancedb => Ok(Self::Lance(lance::LanceStore::open(dir, dims).await?)),
            _ => Ok(Self::Arrow(arrow::ArrowStore::open(dir, dims)?)),
        }
    }

    /// Backend name, for status output.
    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Arrow(_) => "arrow",
            #[cfg(feature = "lancedb")]
            Self::Lance(_) => "lancedb",
        }
    }

    /// Embedding dimensions of the vector column, if any.
    pub fn dims(&self) -> Option<usize> {
        match self {
            Self::Arrow(store) => store.dims(),
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.dims(),
        }
    }

    /// Stages the removal of every chunk of `paths`.
    pub async fn delete_paths(&mut self, paths: &[String]) -> Result<()> {
        match self {
            Self::Arrow(store) => {
                store.delete_paths(paths);
                Ok(())
            }
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.delete_paths(paths).await,
        }
    }

    /// Stages new chunks (with vectors when the store has a vector column).
    pub async fn append(&mut self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        match self {
            Self::Arrow(store) => store.append(chunks, vectors),
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.append(chunks, vectors).await,
        }
    }

    /// Publishes staged changes.
    pub async fn commit(&mut self) -> Result<()> {
        match self {
            Self::Arrow(store) => store.commit(),
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.commit().await,
        }
    }

    /// Fetches chunks by id (unknown ids are skipped).
    pub async fn get(&self, ids: &[String]) -> Result<Vec<Chunk>> {
        match self {
            Self::Arrow(store) => store.get(ids),
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.get(ids).await,
        }
    }

    /// Nearest chunks to a normalized query vector, as (id, cosine similarity).
    pub async fn vector_search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        if self.dims().is_none() || k == 0 {
            return Ok(Vec::new());
        }
        match self {
            Self::Arrow(store) => Ok(store.vector_search(query, k)),
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.vector_search(query, k).await,
        }
    }

    /// Number of committed chunks.
    pub async fn count(&self) -> Result<usize> {
        match self {
            Self::Arrow(store) => Ok(store.count()),
            #[cfg(feature = "lancedb")]
            Self::Lance(store) => store.count().await,
        }
    }
}

/// Escapes a string literal for a SQL filter.
#[cfg_attr(not(feature = "lancedb"), allow(dead_code))]
pub(crate) fn sql_list(values: &[String]) -> String {
    values.iter().map(|v| format!("'{}'", v.replace('\'', "''"))).collect::<Vec<_>>().join(", ")
}
