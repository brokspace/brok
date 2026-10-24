//! Indexing and retrieval for Brokspace.
//!
//! Pipeline: walk the workspace (honoring `.gitignore`) → route each file
//! to its partition (root or git submodule) → skip unchanged files (stat,
//! then content hash) → cut AST-aware chunks → embed them locally → write
//! the chunk store + BM25 index. Queries run a federated hybrid search.
//!
//! Optional native backends are behind cargo features (all on by default):
//! `ast` (tree-sitter), `lancedb` (LanceDB store) and `onnx` (local
//! embeddings). Without them brok still works: structural text chunking,
//! memory-mapped Arrow IPC storage and BM25-only search.

pub mod bm25;
pub mod chunk;
pub mod embed;
mod index;
pub mod language;
pub mod partition_index;
pub mod schema;
mod search;
pub mod service;
pub mod store;

pub use embed::{EmbedKind, Embedder, ModelFiles, load_embedder};
pub use index::{Access, Index, IndexReport, PartitionStatus, Progress};
pub use search::SearchQuery;

/// Names of the optional backends compiled into this build.
pub fn compiled_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    if cfg!(feature = "ast") {
        features.push("ast");
    }
    if cfg!(feature = "lancedb") {
        features.push("lancedb");
    }
    if cfg!(feature = "onnx") {
        features.push("onnx");
    }
    for (enabled, name) in [
        (cfg!(feature = "cuda"), "cuda"),
        (cfg!(feature = "coreml"), "coreml"),
        (cfg!(feature = "directml"), "directml"),
    ] {
        if enabled {
            features.push(name);
        }
    }
    features
}
