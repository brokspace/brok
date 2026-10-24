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
use crate::schema::{VECTOR_COLUMN, chunk_schema, from_record_batch, schema_dims, to_record_batch};

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
    writes_since_optimize: usize,
}

impl LanceStore {
    /// Opens or creates the `chunks` table in `dir`.
    pub async fn open(dir: &Path, dims: Option<usize>) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let uri = dir.to_str().context("database path is not valid UTF-8")?;
        let db = lancedb::connect(uri).execute().await.context("opening LanceDB")?;
        // Existing tables keep their own layout; `dims` only applies to new tables.
        let (table, dims) = if db.table_names().execute().await?.iter().any(|name| name == TABLE) {
            let table = db.open_table(TABLE).execute().await?;
            let stored = schema_dims(table.schema().await?.as_ref());
            (table, stored)
        } else {
            (db.create_empty_table(TABLE, chunk_schema(dims)).execute().await?, dims)
        };
        Ok(Self { table, dims, writes_since_optimize: 0 })
    }

    pub(crate) fn dims(&self) -> Option<usize> {
        self.dims
    }

    pub(crate) async fn delete_paths(&mut self, paths: &[String]) -> Result<()> {
        for batch in paths.chunks(FILTER_BATCH) {
            self.table.delete(&format!("path IN ({})", sql_list(batch))).await?;
            self.writes_since_optimize += 1;
        }
        Ok(())
    }

    pub(crate) async fn append(&mut self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<()> {
        let batch = to_record_batch(chunks, vectors, self.dims)?;
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
