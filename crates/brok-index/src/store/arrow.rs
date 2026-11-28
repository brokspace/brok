//! Memory-mapped Arrow IPC chunk store.
//!
//! Layout of a partition directory:
//!
//! ```text
//! CURRENT                 generation number of the live file
//! chunks-00000007.arrow   immutable Arrow IPC file (generation 7)
//! ```
//!
//! Generations are never modified after being written, which makes memory
//! mapping them sound and lets concurrent readers (e.g. `brok search` while
//! `brok run` indexes) keep using an old generation until they reopen.
//! Record batches are decoded zero-copy straight from the mapping, so the
//! resident memory is whatever pages the OS decides to keep around.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use arrow_array::{BooleanArray, RecordBatch};
use arrow_buffer::Buffer;
use arrow_ipc::convert::fb_to_schema;
use arrow_ipc::reader::{FileDecoder, read_footer_length};
use arrow_ipc::root_as_footer;
use arrow_ipc::writer::FileWriter;

use crate::chunk::Chunk;
use crate::schema::{ChunkColumns, VectorFormat, chunk_schema, schema_dims, schema_format, to_record_batch};

pub(crate) const CURRENT: &str = "CURRENT";
/// Batches are coalesced on commit once there are more than this many.
const MAX_BATCHES: usize = 8;
/// Generations a reader tries before giving up on a very busy writer.
const READ_ATTEMPTS: usize = 100;

/// See the module documentation.
pub struct ArrowStore {
    dir: PathBuf,
    dims: Option<usize>,
    format: VectorFormat,
    generation: u64,
    batches: Vec<RecordBatch>,
    ids: HashMap<String, (usize, usize)>,
    pending_deletes: HashSet<String>,
    pending_appends: Vec<RecordBatch>,
}

impl ArrowStore {
    /// Opens the live generation in `dir`, creating the directory if needed.
    pub fn open(dir: &Path, dims: Option<usize>) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let (generation, batches) = read_live(dir, read_ipc)?;
        // Existing data keeps its own layout; `dims` only applies to new stores.
        let dims = batches.first().map_or(dims, |first| schema_dims(&first.schema()));
        let format = batches.first().and_then(|first| schema_format(&first.schema())).unwrap_or_default();
        let mut store = Self {
            dir: dir.to_path_buf(),
            dims,
            format,
            generation,
            batches,
            ids: HashMap::new(),
            pending_deletes: HashSet::new(),
            pending_appends: Vec::new(),
        };
        store.rebuild_id_map()?;
        Ok(store)
    }

    pub(crate) fn dims(&self) -> Option<usize> {
        self.dims
    }

    /// Makes an empty store write another format, as older versions did.
    #[cfg(test)]
    pub(crate) fn set_format(&mut self, format: VectorFormat) {
        assert!(self.batches.is_empty(), "the store already has a format");
        self.format = format;
    }

    pub(crate) fn format(&self) -> VectorFormat {
        self.format
    }

    pub(crate) fn count(&self) -> usize {
        self.ids.len()
    }

    pub(crate) fn delete_paths(&mut self, paths: &[String]) {
        self.pending_deletes.extend(paths.iter().cloned());
    }

    pub(crate) fn append(&mut self, chunks: &[Chunk], vectors: Option<&[Vec<f32>]>) -> Result<()> {
        self.pending_appends.push(to_record_batch(chunks, vectors, self.dims, self.format)?);
        Ok(())
    }

    pub(crate) fn commit(&mut self) -> Result<()> {
        if self.pending_deletes.is_empty() && self.pending_appends.is_empty() {
            return Ok(());
        }
        let mut next: Vec<RecordBatch> = Vec::with_capacity(self.batches.len() + self.pending_appends.len());
        for batch in &self.batches {
            let columns = ChunkColumns::new(batch)?;
            let keep: BooleanArray =
                (0..columns.len()).map(|row| Some(!self.pending_deletes.contains(columns.path(row)))).collect();
            if keep.true_count() == batch.num_rows() {
                next.push(batch.clone());
            } else if keep.true_count() > 0 {
                next.push(arrow_select::filter::filter_record_batch(batch, &keep)?);
            }
        }
        next.append(&mut self.pending_appends);
        if next.len() > MAX_BATCHES {
            let schema = chunk_schema(self.dims, self.format);
            next = vec![arrow_select::concat::concat_batches(&schema, &next)?];
        }

        let generation = self.generation + 1;
        write_ipc(&generation_path(&self.dir, generation), &chunk_schema(self.dims, self.format), &next)?;
        brok_core::fsutil::write_atomic(&self.dir.join(CURRENT), generation.to_string().as_bytes())?;

        // Release the old mapping before reopening, then garbage collect.
        drop(next);
        self.batches.clear();
        self.pending_deletes.clear();
        self.generation = generation;
        self.batches = read_ipc(&generation_path(&self.dir, generation))?;
        self.rebuild_id_map()?;
        self.remove_stale_generations();
        Ok(())
    }

    pub(crate) fn get(&self, ids: &[String]) -> Result<Vec<Chunk>> {
        let mut chunks = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(&(batch, row)) = self.ids.get(id) {
                chunks.push(ChunkColumns::new(&self.batches[batch])?.chunk(row));
            }
        }
        Ok(chunks)
    }

    pub(crate) fn vector_search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        let mut scored: Vec<(f32, usize, usize)> = Vec::new();
        for (b, batch) in self.batches.iter().enumerate() {
            let Ok(columns) = ChunkColumns::new(batch) else { continue };
            for row in 0..columns.len() {
                if let Some(similarity) = columns.similarity(row, query) {
                    scored.push((similarity, b, row));
                }
            }
        }
        if scored.len() > k {
            scored.select_nth_unstable_by(k, |a, b| b.0.total_cmp(&a.0));
            scored.truncate(k);
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .filter_map(|(score, b, row)| {
                ChunkColumns::new(&self.batches[b]).ok().map(|columns| (columns.id(row).to_owned(), score))
            })
            .collect()
    }

    fn rebuild_id_map(&mut self) -> Result<()> {
        self.ids.clear();
        for (b, batch) in self.batches.iter().enumerate() {
            let columns = ChunkColumns::new(batch)?;
            for row in 0..columns.len() {
                self.ids.insert(columns.id(row).to_owned(), (b, row));
            }
        }
        Ok(())
    }

    fn remove_stale_generations(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let live = generation_path(&self.dir, self.generation);
        for entry in entries.flatten() {
            let path = entry.path();
            let is_generation = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("chunks-"));
            if is_generation && path != live {
                // May fail on Windows while another process maps it; retried next commit.
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn generation_path(dir: &Path, generation: u64) -> PathBuf {
    dir.join(format!("chunks-{generation:08}.arrow"))
}

fn write_ipc(path: &Path, schema: &arrow_schema::Schema, batches: &[RecordBatch]) -> Result<()> {
    let tmp = path.with_extension("arrow.tmp");
    let file = File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    let mut writer = FileWriter::try_new(BufWriter::new(file), schema)?;
    for batch in batches {
        writer.write(batch)?;
    }
    writer.finish()?;
    let file = writer.into_inner()?.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path).with_context(|| format!("publishing {}", path.display()))?;
    Ok(())
}

/// Maps an IPC file and decodes its batches without copying.
/// The generation `CURRENT` names, 0 before the first commit.
fn current_generation(dir: &Path) -> u64 {
    std::fs::read_to_string(dir.join(CURRENT)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// Reads the live generation with `read`. A writer removes a generation as
/// soon as it has replaced it, possibly between reading `CURRENT` and opening
/// the file: then the newer one is read.
fn read_live(dir: &Path, mut read: impl FnMut(&Path) -> Result<Vec<RecordBatch>>) -> Result<(u64, Vec<RecordBatch>)> {
    let mut generation = current_generation(dir);
    let mut attempts = 1;
    loop {
        if generation == 0 {
            return Ok((generation, Vec::new()));
        }
        match read(&generation_path(dir, generation)) {
            Ok(batches) => return Ok((generation, batches)),
            Err(e) => {
                let newer = current_generation(dir);
                // Unchanged: the live file itself cannot be read.
                if newer == generation || attempts == READ_ATTEMPTS {
                    return Err(e);
                }
                generation = newer;
                attempts += 1;
            }
        }
    }
}

fn read_ipc(path: &Path) -> Result<Vec<RecordBatch>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    // SAFETY: generation files are written once, renamed into place and
    // never modified afterwards (see module docs), so the mapping cannot
    // change underneath us.
    #[allow(unsafe_code)]
    let mmap = unsafe { memmap2::Mmap::map(&file) }.with_context(|| format!("mapping {}", path.display()))?;
    let buffer = Buffer::from(bytes::Bytes::from_owner(mmap));

    ensure!(buffer.len() >= 10, "{} is not an Arrow IPC file", path.display());
    let trailer_start = buffer.len() - 10;
    let footer_len = read_footer_length(buffer[trailer_start..].try_into()?)?;
    let footer = root_as_footer(&buffer[trailer_start - footer_len..trailer_start])
        .map_err(|e| anyhow::anyhow!("corrupt Arrow footer in {}: {e}", path.display()))?;
    let schema = fb_to_schema(footer.schema().context("Arrow file without schema")?);
    let mut decoder = FileDecoder::new(Arc::new(schema), footer.version());

    for block in footer.dictionaries().iter().flatten() {
        let len = block.bodyLength() as usize + block.metaDataLength() as usize;
        decoder.read_dictionary(block, &buffer.slice_with_length(block.offset() as usize, len))?;
    }
    let mut batches = Vec::new();
    for block in footer.recordBatches().iter().flatten() {
        let len = block.bodyLength() as usize + block.metaDataLength() as usize;
        if let Some(batch) =
            decoder.read_record_batch(block, &buffer.slice_with_length(block.offset() as usize, len))?
        {
            batches.push(batch);
        }
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Precision;
    use crate::schema::tests::sample;

    #[test]
    fn a_generation_replaced_while_opening_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let live = |n: u64| std::fs::write(dir.path().join(CURRENT), n.to_string()).unwrap();
        live(1);
        let mut tried = Vec::new();
        let (generation, _) = read_live(dir.path(), |path| {
            tried.push(path.to_path_buf());
            if tried.len() == 1 {
                live(2);
                anyhow::bail!("removed by the writer");
            }
            Ok(Vec::new())
        })
        .unwrap();
        assert_eq!(generation, 2);
        assert_eq!(tried, [generation_path(dir.path(), 1), generation_path(dir.path(), 2)]);

        // A live file that cannot be read stays an error, without retrying.
        let mut reads = 0;
        let broken = read_live(dir.path(), |_| {
            reads += 1;
            anyhow::bail!("damaged")
        });
        assert!(broken.is_err());
        assert_eq!(reads, 1);
    }

    #[test]
    fn commit_delete_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ArrowStore::open(dir.path(), Some(2)).unwrap();
        let chunks = vec![sample("a1", "a.rs"), sample("b1", "b.rs"), sample("b2", "b.rs")];
        let vectors = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.6, 0.8]];
        store.append(&chunks, Some(&vectors)).unwrap();
        store.commit().unwrap();
        assert_eq!(store.count(), 3);

        let hits = store.vector_search(&[0.0, 1.0], 2);
        assert_eq!(hits.iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), vec!["b1", "b2"]);

        store.delete_paths(&["b.rs".to_string()]);
        store.commit().unwrap();
        assert_eq!(store.count(), 1);
        assert_eq!(store.get(&["a1".into(), "b1".into()]).unwrap().len(), 1);

        let reopened = ArrowStore::open(dir.path(), Some(2)).unwrap();
        assert_eq!(reopened.count(), 1);
        let generations = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("chunks-"))
            .count();
        assert_eq!(generations, 1, "stale generations are garbage collected");

        assert_eq!(ArrowStore::open(dir.path(), Some(3)).unwrap().dims(), Some(2), "stored layout wins");
    }

    #[test]
    fn a_new_store_is_half_precision_and_an_old_one_stays_single() {
        let vectors = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let chunks = [sample("a1", "a.rs"), sample("b1", "b.rs")];

        let new = tempfile::tempdir().unwrap();
        let mut store = ArrowStore::open(new.path(), Some(2)).unwrap();
        store.append(&chunks, Some(&vectors)).unwrap();
        store.commit().unwrap();
        assert_eq!(ArrowStore::open(new.path(), Some(2)).unwrap().format(), VectorFormat::default());

        // An index written by an earlier brok.
        let old = tempfile::tempdir().unwrap();
        let single = VectorFormat { precision: Precision::Single, optional: false };
        let batch = to_record_batch(&chunks, Some(&vectors), Some(2), single).unwrap();
        write_ipc(&generation_path(old.path(), 1), &batch.schema(), std::slice::from_ref(&batch)).unwrap();
        std::fs::write(old.path().join(CURRENT), "1").unwrap();
        let mut store = ArrowStore::open(old.path(), Some(2)).unwrap();
        assert_eq!(store.format(), single);
        store.append(&[sample("c1", "c.rs")], Some(&[vec![0.6, 0.8]])).unwrap();
        store.commit().unwrap();
        let reopened = ArrowStore::open(old.path(), Some(2)).unwrap();
        assert_eq!((reopened.format(), reopened.count()), (single, 3));
        assert_eq!(reopened.vector_search(&[0.6, 0.8], 1)[0].0, "c1");
    }

    #[test]
    fn rows_waiting_for_vectors_are_kept_and_not_matched() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ArrowStore::open(dir.path(), Some(2)).unwrap();
        store.append(&[sample("a1", "a.rs"), sample("b1", "b.rs")], None).unwrap();
        store.commit().unwrap();
        let mut store = ArrowStore::open(dir.path(), Some(2)).unwrap();
        assert_eq!(store.count(), 2);
        assert!(store.vector_search(&[1.0, 0.0], 5).is_empty());
        store.delete_paths(&["a.rs".to_string()]);
        store.append(&[sample("a1", "a.rs")], Some(&[vec![1.0, 0.0]])).unwrap();
        store.commit().unwrap();
        let found = store.vector_search(&[1.0, 0.0], 5);
        assert_eq!((store.count(), found.len(), found[0].0.as_str()), (2, 1, "a1"));
    }

    #[test]
    fn many_appends_are_coalesced() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ArrowStore::open(dir.path(), None).unwrap();
        for i in 0..20 {
            store.append(&[sample(&format!("c{i}"), &format!("f{i}.rs"))], None).unwrap();
        }
        store.commit().unwrap();
        assert_eq!(store.batches.len(), 1);
        assert_eq!(store.count(), 20);
    }
}
