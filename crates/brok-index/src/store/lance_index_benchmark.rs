//! The LanceDB vector index against a full scan: latency, recall and disk
//! (brokspace/brok#192). Real embeddings, 768 little endian `f32` values per
//! vector, are mixed with noise to reach each size.
//!
//! ```sh
//! BROK_BENCH_VECTORS=vectors.f32 cargo test --release -p brok-index measure_index -- --ignored --nocapture
//! ```

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use futures::TryStreamExt;
use lancedb::DistanceType;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::table::OptimizeAction;

use super::LanceStore;
use crate::schema::{VECTOR_COLUMN, tests::sample};

const DIMS: usize = 768;
const K: usize = 32;

fn size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).into_iter().flatten().flatten().fold(0, |total, entry| {
        let m = entry.metadata().unwrap();
        total + if m.is_dir() { size(&entry.path()) } else { m.len() }
    })
}

/// Mixtures of real embeddings with noise, to reach a size with a realistic shape.
fn mixtures(real: &[f32], count: usize, seed: u64) -> Vec<Vec<f32>> {
    let rows: Vec<&[f32]> = real.chunks(DIMS).collect();
    let mut state = seed;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count)
        .map(|_| {
            let (a, b) = (rows[next() as usize % rows.len()], rows[next() as usize % rows.len()]);
            let w = (next() % 1000) as f32 / 1000.0;
            let mut v: Vec<f32> =
                (0..DIMS).map(|d| a[d] + w * b[d] + 0.012 * ((next() % 2000) as f32 / 1000.0 - 1.0)).collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter_mut().for_each(|x| *x /= norm);
            v
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark: it needs a file of real vectors"]
async fn measure_index() {
    let bytes = std::fs::read(std::env::var("BROK_BENCH_VECTORS").unwrap()).unwrap();
    let real: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let queries = mixtures(&real, 50, 7);
    for count in [20_000usize, 100_000, 400_000] {
        let vectors = mixtures(&real, count, 99);
        let exact: Vec<HashSet<String>> = queries
            .iter()
            .map(|q| {
                let mut scored: Vec<(f32, usize)> =
                    vectors.iter().enumerate().map(|(i, v)| (v.iter().zip(q).map(|(a, b)| a * b).sum(), i)).collect();
                scored.select_nth_unstable_by(K, |a, b| b.0.total_cmp(&a.0));
                scored[..K].iter().map(|(_, i)| format!("c{i}")).collect()
            })
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let mut store = LanceStore::open(dir.path(), Some(DIMS)).await.unwrap();
        for (n, group) in vectors.chunks(5_000).enumerate() {
            let chunks: Vec<_> = (0..group.len()).map(|i| sample(&format!("c{}", n * 5_000 + i), "a.rs")).collect();
            store.append(&chunks, Some(group)).await.unwrap();
        }
        store.table.optimize(OptimizeAction::All).await.unwrap();
        let before = size(dir.path());
        let started = Instant::now();
        store
            .table
            .create_index(
                &[VECTOR_COLUMN],
                Index::IvfPq(IvfPqIndexBuilder::default().distance_type(DistanceType::Cosine)),
            )
            .execute()
            .await
            .unwrap();
        println!(
            "\n{count} chunks: data {:.0} MB, index +{:.0} MB, built in {:.1}s",
            before as f64 / 1e6,
            (size(dir.path()) - before) as f64 / 1e6,
            started.elapsed().as_secs_f64()
        );
        println!("| search | p50 ms | p95 ms | recall@{K} | worst query |");
        println!("|---|---:|---:|---:|---:|");
        // (nprobes, refine factor); None is the full scan.
        let variants = [
            None,
            Some((None, None)),
            Some((Some(20), None)),
            Some((None, Some(10))),
            Some((Some(20), Some(10))),
            Some((Some(50), Some(10))),
            Some((Some(50), Some(30))),
        ];
        for variant in variants {
            let mut times = Vec::new();
            let mut recalls = Vec::new();
            for (q, exact) in queries.iter().zip(&exact) {
                let started = Instant::now();
                let mut query = store
                    .table
                    .query()
                    .nearest_to(q.as_slice())
                    .unwrap()
                    .distance_type(DistanceType::Cosine)
                    .select(Select::columns(&["id", "_distance"]))
                    .limit(K);
                match variant {
                    None => query = query.bypass_vector_index(),
                    Some((probes, refine)) => {
                        if let Some(p) = probes {
                            query = query.nprobes(p).unwrap();
                        }
                        if let Some(r) = refine {
                            query = query.refine_factor(r);
                        }
                    }
                }
                let batches: Vec<RecordBatch> = query.execute().await.unwrap().try_collect().await.unwrap();
                times.push(started.elapsed().as_secs_f64() * 1e3);
                let found = batches
                    .iter()
                    .flat_map(|b| {
                        let ids = b.column_by_name("id").unwrap().as_string::<i32>();
                        (0..b.num_rows()).map(|r| ids.value(r).to_owned()).collect::<Vec<_>>()
                    })
                    .filter(|id| exact.contains(id))
                    .count();
                recalls.push(found as f64 / K as f64);
            }
            times.sort_by(f64::total_cmp);
            recalls.sort_by(f64::total_cmp);
            let name = match variant {
                None => "full scan".to_owned(),
                Some((p, r)) => format!("index, nprobes {p:?}, refine {r:?}"),
            };
            println!(
                "| {name} | {:.1} | {:.1} | {:.1}% | {:.0}% |",
                times[25],
                times[47],
                100.0 * recalls.iter().sum::<f64>() / 50.0,
                100.0 * recalls[0]
            );
        }
    }
}
