//! LanceDB chunk store.

use std::path::Path;

use anyhow::{Context, Result};
use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
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
        if self.dims.is_some() && self.count().await? >= ANN_THRESHOLD {
            let indexed = self.table.list_indices().await?.iter().any(|i| i.columns.iter().any(|c| c == VECTOR_COLUMN));
            if !indexed {
                tracing::info!("building ANN vector index");
                self.table.create_index(&[VECTOR_COLUMN], Index::Auto).execute().await?;
            }
        }
        Ok(())
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
        let batches: Vec<RecordBatch> = self
            .table
            .query()
            .nearest_to(query)?
            .distance_type(DistanceType::Cosine)
            .select(Select::columns(&["id", "_distance"]))
            .limit(k)
            .execute()
            .await?
            .try_collect()
            .await?;
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
}
