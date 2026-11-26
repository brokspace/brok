//! LanceDB chunk store.

use std::path::Path;

use anyhow::{Context, Result};
use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase, Select, VectorQuery};
use lancedb::table::OptimizeAction;
use lancedb::{DistanceType, Table};

use super::sql_list;
use crate::chunk::Chunk;
use crate::schema::{
    Precision, VECTOR_COLUMN, chunk_schema, from_record_batch, schema_dims, schema_precision, to_record_batch,
};

const TABLE: &str = "chunks";
/// Rows from which an ANN index pays off over brute force.
const ANN_THRESHOLD: usize = 10_000;
/// Metric of both the ANN index and the queries: LanceDB ignores an index
/// built with another one and scans every row.
const METRIC: DistanceType = DistanceType::Cosine;
/// The index compares compressed vectors, so this many times the wanted
/// results are re-ranked with the stored ones (docs/benchmarks.md).
const REFINE: u32 = 10;
/// Fragments written before compaction runs.
const COMPACT_EVERY: usize = 16;
/// Filter list size per SQL statement.
const FILTER_BATCH: usize = 256;

/// LanceDB-backed store.
pub struct LanceStore {
    table: Table,
    dims: Option<usize>,
    precision: Precision,
    writes_since_optimize: usize,
}

impl LanceStore {
    /// Opens or creates the `chunks` table in `dir`.
    pub async fn open(dir: &Path, dims: Option<usize>) -> Result<Self> {
        Self::open_as(dir, dims, Precision::default()).await
    }

    /// [`Self::open`], creating a new table with vectors of type `new`.
    pub(crate) async fn open_as(dir: &Path, dims: Option<usize>, new: Precision) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let uri = dir.to_str().context("database path is not valid UTF-8")?;
        let db = lancedb::connect(uri).execute().await.context("opening LanceDB")?;
        // Existing tables keep their own layout; `dims` only applies to new tables.
        let (table, dims, precision) = if db.table_names().execute().await?.iter().any(|name| name == TABLE) {
            let table = db.open_table(TABLE).execute().await?;
            let schema = table.schema().await?;
            let precision = schema_precision(schema.as_ref()).unwrap_or_default();
            (table, schema_dims(schema.as_ref()), precision)
        } else {
            (db.create_empty_table(TABLE, chunk_schema(dims, new)).execute().await?, dims, new)
        };
        Ok(Self { table, dims, precision, writes_since_optimize: 0 })
    }

    pub(crate) fn dims(&self) -> Option<usize> {
        self.dims
    }

    pub(crate) fn precision(&self) -> Precision {
        self.precision
    }

    pub(crate) async fn delete_paths(&mut self, paths: &[String]) -> Result<()> {
        for batch in paths.chunks(FILTER_BATCH) {
            self.table.delete(&format!("path IN ({})", sql_list(batch))).await?;
            self.writes_since_optimize += 1;
        }
        Ok(())
    }

    pub(crate) async fn append(&mut self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<()> {
        let batch = to_record_batch(chunks, vectors, self.dims, self.precision)?;
        self.table.add(batch).execute().await?;
        self.writes_since_optimize += 1;
        Ok(())
    }

    pub(crate) async fn commit(&mut self) -> Result<()> {
        if self.writes_since_optimize >= COMPACT_EVERY {
            self.table.optimize(OptimizeAction::All).await?;
            self.writes_since_optimize = 0;
        }
        if self.dims.is_some() && self.count().await? >= ANN_THRESHOLD && !self.has_usable_index().await? {
            tracing::info!("building ANN vector index");
            // Replaces an index brok 0.11 and earlier built with another metric.
            let index = Index::IvfPq(IvfPqIndexBuilder::default().distance_type(METRIC));
            self.table.create_index(&[VECTOR_COLUMN], index).execute().await?;
        }
        Ok(())
    }

    /// Whether the vectors have an index that [`Self::nearest`] queries use.
    async fn has_usable_index(&self) -> Result<bool> {
        for index in self.table.list_indices().await? {
            if index.columns.iter().any(|c| c == VECTOR_COLUMN) {
                let stats = self.table.index_stats(&index.name).await?;
                return Ok(stats.and_then(|s| s.distance_type) == Some(METRIC));
            }
        }
        Ok(false)
    }

    /// The query for the `k` chunks nearest to `query`.
    fn nearest(&self, query: &[f32], k: usize) -> Result<VectorQuery> {
        Ok(self
            .table
            .query()
            .nearest_to(query)?
            .distance_type(METRIC)
            .refine_factor(REFINE)
            .select(Select::columns(&["id", "_distance"]))
            .limit(k))
    }

    pub(crate) async fn get(&self, ids: &[String]) -> Result<Vec<Chunk>> {
        let mut chunks = Vec::with_capacity(ids.len());
        for batch in ids.chunks(FILTER_BATCH) {
            let batches: Vec<RecordBatch> = self
                .table
                .query()
                .only_if(format!("id IN ({})", sql_list(batch)))
                .execute()
                .await?
                .try_collect()
                .await?;
            for batch in &batches {
                chunks.extend(from_record_batch(batch)?);
            }
        }
        Ok(chunks)
    }

    pub(crate) async fn vector_search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        if self.count().await? == 0 {
            return Ok(Vec::new());
        }
        let batches: Vec<RecordBatch> = self.nearest(query, k)?.execute().await?.try_collect().await?;
        let mut hits = Vec::with_capacity(k);
        for batch in &batches {
            let ids = batch.column_by_name("id").and_then(|c| c.as_string_opt::<i32>()).context("missing id")?;
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_primitive_opt::<Float32Type>())
                .context("missing _distance")?;
            for row in 0..batch.num_rows() {
                hits.push((ids.value(row).to_owned(), 1.0 - distances.value(row)));
            }
        }
        Ok(hits)
    }

    pub(crate) async fn count(&self) -> Result<usize> {
        Ok(self.table.count_rows(None).await?)
    }
}

#[cfg(test)]
#[path = "lance_index_benchmark.rs"]
mod benchmark;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::tests::sample;

    #[tokio::test]
    async fn a_new_table_is_half_precision_and_an_old_one_stays_single() {
        let chunks = [sample("a1", "a.rs"), sample("b1", "b.rs")];
        let vectors = [vec![1.0, 0.0], vec![0.0, 1.0]];
        for old in [Precision::Single, Precision::Half] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = LanceStore::open_as(dir.path(), Some(2), old).await.unwrap();
            store.append(&chunks, Some(&vectors)).await.unwrap();
            store.commit().await.unwrap();
            drop(store);
            // Reopened by this brok: the table keeps its type and takes more rows.
            let mut store = LanceStore::open(dir.path(), Some(2)).await.unwrap();
            assert_eq!(store.precision(), old);
            store.append(&[sample("c1", "c.rs")], Some(&[vec![0.6, 0.8]])).await.unwrap();
            store.commit().await.unwrap();
            let hits = store.vector_search(&[0.6, 0.8], 1).await.unwrap();
            assert_eq!(hits[0].0, "c1");
            assert!((hits[0].1 - 1.0).abs() < 1e-3, "{hits:?}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(LanceStore::open(dir.path(), Some(2)).await.unwrap().precision(), Precision::Half);
    }

    const DIMS: usize = 32;

    /// Distinct, normalized and deterministic.
    fn vector(seed: usize) -> Vec<f32> {
        let mut state = (seed as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let raw: Vec<f32> = (0..DIMS)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % 2000) as f32 / 1000.0 - 1.0
            })
            .collect();
        let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
        raw.into_iter().map(|v| v / norm).collect()
    }

    async fn add(store: &mut LanceStore, ids: std::ops::Range<usize>) {
        let chunks: Vec<_> = ids.clone().map(|i| sample(&format!("c{i}"), &format!("f{i}.rs"))).collect();
        let vectors: Vec<_> = ids.map(vector).collect();
        store.append(&chunks, Some(&vectors)).await.unwrap();
        store.commit().await.unwrap();
    }

    async fn plan(store: &LanceStore) -> String {
        store.nearest(&vector(1), 5).unwrap().explain_plan(true).await.unwrap()
    }

    async fn first(store: &LanceStore, seed: usize) -> (String, f32) {
        store.vector_search(&vector(seed), 5).await.unwrap().remove(0)
    }

    #[tokio::test]
    async fn searches_use_the_index_and_see_later_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LanceStore::open(dir.path(), Some(DIMS)).await.unwrap();
        add(&mut store, 0..ANN_THRESHOLD - 1).await;
        assert!(!plan(&store).await.contains("ANNSubIndex"), "a small table is scanned");
        add(&mut store, ANN_THRESHOLD - 1..ANN_THRESHOLD).await;
        let plan_now = plan(&store).await;
        assert!(plan_now.contains("ANNSubIndex"), "{plan_now}");
        let (id, similarity) = first(&store, 1234).await;
        assert_eq!(id, "c1234");
        assert!((similarity - 1.0).abs() < 1e-3, "the similarity is exact: {similarity}");

        // Rows added and removed after the index was built.
        add(&mut store, ANN_THRESHOLD..ANN_THRESHOLD + 20).await;
        assert_eq!(first(&store, ANN_THRESHOLD + 7).await.0, format!("c{}", ANN_THRESHOLD + 7));
        store.delete_paths(&["f1234.rs".to_string()]).await.unwrap();
        store.commit().await.unwrap();
        assert!(store.vector_search(&vector(1234), 20).await.unwrap().iter().all(|(id, _)| id != "c1234"));
    }

    #[tokio::test]
    async fn an_index_with_the_wrong_metric_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LanceStore::open(dir.path(), Some(DIMS)).await.unwrap();
        add(&mut store, 0..ANN_THRESHOLD - 1).await;
        add(&mut store, ANN_THRESHOLD - 1..ANN_THRESHOLD).await;
        // What brok 0.11 and earlier built.
        let old = Index::IvfPq(IvfPqIndexBuilder::default().distance_type(DistanceType::L2));
        store.table.create_index(&[VECTOR_COLUMN], old).execute().await.unwrap();
        assert!(!plan(&store).await.contains("ANNSubIndex"), "ignored, as reported");

        add(&mut store, ANN_THRESHOLD..ANN_THRESHOLD + 1).await;
        assert!(plan(&store).await.contains("ANNSubIndex"));
        assert_eq!(store.table.list_indices().await.unwrap().len(), 1);
    }
}
