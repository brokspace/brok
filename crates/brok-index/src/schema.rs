//! The Apache Arrow schema of the chunk table.
//!
//! The same schema backs both stores (LanceDB tables and Arrow IPC files),
//! so chunks keep every piece of metadata needed to cite them: file path,
//! line and byte ranges, symbol, enclosing symbol and signature.
//!
//! | column       | type                        | nullable |
//! |--------------|-----------------------------|----------|
//! | `id`         | Utf8                        | no       |
//! | `path`       | Utf8                        | no       |
//! | `language`   | Utf8                        | no       |
//! | `kind`       | Utf8                        | no       |
//! | `symbol`     | Utf8                        | yes      |
//! | `parent`     | Utf8                        | yes      |
//! | `signature`  | Utf8                        | yes      |
//! | `start_line` | UInt32                      | no       |
//! | `end_line`   | UInt32                      | no       |
//! | `start_byte` | UInt32                      | no       |
//! | `end_byte`   | UInt32                      | no       |
//! | `file_hash`  | Utf8                        | no       |
//! | `content`    | Utf8                        | no       |
//! | `vector`     | FixedSizeList<Float32, dim> | no (only when embeddings are on) |

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, ensure};
use arrow_array::cast::AsArray;
use arrow_array::types::{Float32Type, UInt32Type};
use arrow_array::{Array, ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema, SchemaRef};

use crate::chunk::{Chunk, ChunkKind};

/// Bumped whenever the column layout changes; stores refuse mismatches.
pub const SCHEMA_VERSION: &str = "1";
/// Name of the embedding column.
pub const VECTOR_COLUMN: &str = "vector";

const META_VERSION: &str = "brok.schema_version";
const META_DIMS: &str = "brok.embedding_dims";

/// Builds the chunk schema, with a vector column when `dims` is set.
pub fn chunk_schema(dims: Option<usize>) -> SchemaRef {
    let utf8 = |name: &str, nullable| Field::new(name, DataType::Utf8, nullable);
    let u32_field = |name: &str| Field::new(name, DataType::UInt32, false);
    let mut fields = vec![
        utf8("id", false),
        utf8("path", false),
        utf8("language", false),
        utf8("kind", false),
        utf8("symbol", true),
        utf8("parent", true),
        utf8("signature", true),
        u32_field("start_line"),
        u32_field("end_line"),
        u32_field("start_byte"),
        u32_field("end_byte"),
        utf8("file_hash", false),
        utf8("content", false),
    ];
    let mut metadata = HashMap::from([(META_VERSION.to_owned(), SCHEMA_VERSION.to_owned())]);
    if let Some(dims) = dims {
        fields.push(Field::new(VECTOR_COLUMN, vector_type(dims), false));
        metadata.insert(META_DIMS.to_owned(), dims.to_string());
    }
    Arc::new(Schema::new_with_metadata(fields, metadata))
}

fn vector_type(dims: usize) -> DataType {
    DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dims as i32)
}

/// Embedding dimensions declared by a schema (`None` = no vector column).
pub fn schema_dims(schema: &Schema) -> Option<usize> {
    match schema.field_with_name(VECTOR_COLUMN).ok()?.data_type() {
        DataType::FixedSizeList(_, dims) => usize::try_from(*dims).ok(),
        _ => None,
    }
}

/// Converts chunks (and their vectors) into a record batch.
pub fn to_record_batch(chunks: &[Chunk], vectors: Option<&[Vec<f32>]>, dims: Option<usize>) -> Result<RecordBatch> {
    let schema = chunk_schema(dims);
    let strings = |f: fn(&Chunk) -> &str| -> ArrayRef { Arc::new(StringArray::from_iter_values(chunks.iter().map(f))) };
    let optional =
        |f: fn(&Chunk) -> Option<&str>| -> ArrayRef { Arc::new(chunks.iter().map(f).collect::<StringArray>()) };
    let numbers = |f: fn(&Chunk) -> u32| -> ArrayRef { Arc::new(UInt32Array::from_iter_values(chunks.iter().map(f))) };

    let mut columns: Vec<ArrayRef> = vec![
        strings(|c| &c.id),
        strings(|c| &c.path),
        strings(|c| &c.language),
        strings(|c| c.kind.as_str()),
        optional(|c| c.symbol.as_deref()),
        optional(|c| c.parent.as_deref()),
        optional(|c| c.signature.as_deref()),
        numbers(|c| c.start_line),
        numbers(|c| c.end_line),
        numbers(|c| c.start_byte),
        numbers(|c| c.end_byte),
        strings(|c| &c.file_hash),
        strings(|c| &c.content),
    ];

    if let Some(dims) = dims {
        let vectors = vectors.context("vectors are required when the schema has a vector column")?;
        ensure!(vectors.len() == chunks.len(), "got {} vectors for {} chunks", vectors.len(), chunks.len());
        let mut flat = Vec::with_capacity(vectors.len() * dims);
        for vector in vectors {
            ensure!(vector.len() == dims, "vector has {} dimensions, expected {dims}", vector.len());
            flat.extend_from_slice(vector);
        }
        let item = Arc::new(Field::new("item", DataType::Float32, true));
        let list = FixedSizeListArray::try_new(item, dims as i32, Arc::new(Float32Array::from(flat)), None)?;
        columns.push(Arc::new(list));
    }

    Ok(RecordBatch::try_new(schema, columns)?)
}

/// Typed, zero-copy view over the columns of a chunk batch.
pub struct ChunkColumns<'a> {
    id: &'a StringArray,
    path: &'a StringArray,
    language: &'a StringArray,
    kind: &'a StringArray,
    symbol: &'a StringArray,
    parent: &'a StringArray,
    signature: &'a StringArray,
    start_line: &'a UInt32Array,
    end_line: &'a UInt32Array,
    start_byte: &'a UInt32Array,
    end_byte: &'a UInt32Array,
    file_hash: &'a StringArray,
    content: &'a StringArray,
    vector: Option<&'a FixedSizeListArray>,
}

impl<'a> ChunkColumns<'a> {
    /// Binds the columns of `batch`, failing on schema mismatches.
    pub fn new(batch: &'a RecordBatch) -> Result<Self> {
        let string = |name: &str| -> Result<&'a StringArray> {
            batch.column_by_name(name).and_then(|c| c.as_string_opt::<i32>()).ok_or_else(|| missing(name))
        };
        let number = |name: &str| -> Result<&'a UInt32Array> {
            batch.column_by_name(name).and_then(|c| c.as_primitive_opt::<UInt32Type>()).ok_or_else(|| missing(name))
        };
        Ok(Self {
            id: string("id")?,
            path: string("path")?,
            language: string("language")?,
            kind: string("kind")?,
            symbol: string("symbol")?,
            parent: string("parent")?,
            signature: string("signature")?,
            start_line: number("start_line")?,
            end_line: number("end_line")?,
            start_byte: number("start_byte")?,
            end_byte: number("end_byte")?,
            file_hash: string("file_hash")?,
            content: string("content")?,
            vector: batch.column_by_name(VECTOR_COLUMN).and_then(|c| c.as_fixed_size_list_opt()),
        })
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.id.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }

    /// Chunk id of a row.
    pub fn id(&self, row: usize) -> &'a str {
        self.id.value(row)
    }

    /// File path of a row.
    pub fn path(&self, row: usize) -> &'a str {
        self.path.value(row)
    }

    /// Embedding of a row, borrowed from the underlying buffer.
    pub fn vector(&self, row: usize) -> Option<&'a [f32]> {
        let list = self.vector?;
        let dims = list.value_length() as usize;
        let values = list.values().as_primitive_opt::<Float32Type>()?.values();
        let start = list.value_offset(row) as usize;
        values.get(start..start + dims)
    }

    /// Materializes one row.
    pub fn chunk(&self, row: usize) -> Chunk {
        let opt = |array: &StringArray| (!array.is_null(row)).then(|| array.value(row).to_owned());
        Chunk {
            id: self.id.value(row).to_owned(),
            path: self.path.value(row).to_owned(),
            language: self.language.value(row).to_owned(),
            kind: ChunkKind::parse(self.kind.value(row)),
            symbol: opt(self.symbol),
            parent: opt(self.parent),
            signature: opt(self.signature),
            start_line: self.start_line.value(row),
            end_line: self.end_line.value(row),
            start_byte: self.start_byte.value(row),
            end_byte: self.end_byte.value(row),
            file_hash: self.file_hash.value(row).to_owned(),
            content: self.content.value(row).to_owned(),
        }
    }
}

fn missing(column: &str) -> anyhow::Error {
    anyhow!("chunk table is missing column `{column}` (or it has the wrong type); run `brok index --rebuild`")
}

/// Materializes every row of `batch`.
pub fn from_record_batch(batch: &RecordBatch) -> Result<Vec<Chunk>> {
    let columns = ChunkColumns::new(batch)?;
    Ok((0..columns.len()).map(|row| columns.chunk(row)).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample(id: &str, path: &str) -> Chunk {
        Chunk {
            id: id.into(),
            path: path.into(),
            language: "rust".into(),
            kind: ChunkKind::Function,
            symbol: Some("main".into()),
            parent: None,
            signature: Some("fn main()".into()),
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 20,
            file_hash: "abc".into(),
            content: "fn main() {}".into(),
        }
    }

    #[test]
    fn roundtrip_without_vectors() {
        let chunks = vec![sample("1", "a.rs"), Chunk { symbol: None, ..sample("2", "b.rs") }];
        let batch = to_record_batch(&chunks, None, None).unwrap();
        assert_eq!(schema_dims(&batch.schema()), None);
        assert_eq!(from_record_batch(&batch).unwrap(), chunks);
    }

    #[test]
    fn roundtrip_with_vectors() {
        let chunks = vec![sample("1", "a.rs"), sample("2", "b.rs")];
        let vectors = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let batch = to_record_batch(&chunks, Some(&vectors), Some(3)).unwrap();
        assert_eq!(schema_dims(&batch.schema()), Some(3));
        let columns = ChunkColumns::new(&batch).unwrap();
        assert_eq!(columns.vector(1).unwrap(), &[0.0, 1.0, 0.0]);
        assert!(to_record_batch(&chunks, Some(&vectors[..1]), Some(3)).is_err());
    }
}
