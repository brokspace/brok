//! `float16` against `float32` vectors in each store (ADR 0017): bytes on
//! disk, search latency and how many of the exact `float32` results come back.
//!
//! ```sh
//! cargo test --release -p brok-index precision_benchmark -- --ignored --nocapture
//! ```
//!
//! Vectors are synthetic and clustered. `BROK_BENCH_VECTORS` may name a file
//! of little endian `f32` values, 768 per vector, to use real embeddings.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use brok_core::config::StoreBackend;

use super::{ChunkStore, arrow::ArrowStore};
use crate::chunk::Chunk;
use crate::schema::{Precision, VectorFormat, tests::sample};

const DIMS: usize = 768;
const K: usize = 32;
const QUERIES: usize = 50;

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// Deterministic noise in -1..1.
fn noise(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

fn synthetic(count: usize) -> Vec<Vec<f32>> {
    let mut state = 0x9E37_79B9_7F4A_7C15;
    let centres: Vec<Vec<f32>> = (0..200).map(|_| (0..DIMS).map(|_| noise(&mut state)).collect()).collect();
    (0..count).map(|i| normalize(centres[i % 200].iter().map(|c| c + 0.8 * noise(&mut state)).collect())).collect()
}

fn size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).into_iter().flatten().flatten().fold(0, |total, entry| {
        let metadata = entry.metadata().expect("metadata");
        total + if metadata.is_dir() { size(&entry.path()) } else { metadata.len() }
    })
}

async fn open(dir: &Path, backend: StoreBackend, precision: Precision) -> ChunkStore {
    match backend {
        #[cfg(feature = "lancedb")]
        StoreBackend::Lancedb => ChunkStore::Lance(
            super::lance::LanceStore::open_as(dir, Some(DIMS), VectorFormat { precision, optional: false })
                .await
                .expect("lance"),
        ),
        _ => {
            let mut store = ArrowStore::open(dir, Some(DIMS)).expect("arrow");
            store.set_format(VectorFormat { precision, optional: false });
            ChunkStore::Arrow(store)
        }
    }
}

async fn measure(backend: StoreBackend, vectors: &[Vec<f32>]) {
    let queries: Vec<&Vec<f32>> = vectors.iter().step_by((vectors.len() / QUERIES).max(1)).take(QUERIES).collect();
    let exact: Vec<HashSet<String>> = queries
        .iter()
        .map(|q| {
            let mut scored: Vec<(f32, usize)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (v.iter().zip(q.iter()).map(|(a, b)| a * b).sum(), i))
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            scored.iter().take(K).map(|(_, i)| format!("c{i}")).collect()
        })
        .collect();
    let content = "    let value = compute(input, &config);\n".repeat(30);
    for precision in [Precision::Single, Precision::Half] {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = open(dir.path(), backend, precision).await;
        let started = Instant::now();
        for (n, group) in vectors.chunks(256).enumerate() {
            let chunks: Vec<Chunk> = (0..group.len())
                .map(|i| {
                    let id = format!("c{}", n * 256 + i);
                    Chunk { content: content.clone(), ..sample(&id, &format!("f{n}.rs")) }
                })
                .collect();
            store.append(&chunks, Some(group)).await.expect("append");
        }
        store.commit().await.expect("commit");
        let written = started.elapsed();
        let mut latencies = Vec::new();
        let mut found = 0;
        for (query, exact) in queries.iter().zip(&exact) {
            let started = Instant::now();
            let hits = store.vector_search(query, K).await.expect("search");
            latencies.push(started.elapsed().as_secs_f64() * 1e3);
            found += hits.iter().filter(|(id, _)| exact.contains(id)).count();
        }
        latencies.sort_by(f64::total_cmp);
        println!(
            "| {} | {} | {precision:?} | {:.1} | {:.2} | {:.2} | {:.1}% | {:.1} |",
            store.backend_name(),
            vectors.len(),
            size(dir.path()) as f64 / 1e6,
            latencies[latencies.len() / 2],
            latencies[latencies.len() * 95 / 100],
            100.0 * found as f64 / (K * queries.len()) as f64,
            written.as_secs_f64(),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark: run it in release with --ignored --nocapture"]
async fn precision_benchmark() {
    println!("| store | chunks | vectors | MB on disk | search p50 ms | p95 ms | recall@{K} | write s |");
    println!("|---|---:|---|---:|---:|---:|---:|---:|");
    let mut sets = vec![synthetic(10_000), synthetic(100_000)];
    if let Ok(path) = std::env::var("BROK_BENCH_VECTORS") {
        let bytes = std::fs::read(path).expect("vector file");
        let values: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        sets.insert(0, values.as_chunks::<DIMS>().0.iter().map(|v| v.to_vec()).collect());
    }
    for vectors in &sets {
        measure(StoreBackend::Arrow, vectors).await;
        #[cfg(feature = "lancedb")]
        measure(StoreBackend::Lancedb, vectors).await;
    }
}
