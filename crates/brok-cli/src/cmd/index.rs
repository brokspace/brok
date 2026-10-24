//! `brok index`

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use brok_index::{Access, Index, Progress};
use indicatif::ProgressBar;

use crate::ui;

/// Arguments of `brok index`.
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Drop the existing index and rebuild everything.
    #[arg(long)]
    pub rebuild: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let embedder = brok_index::load_embedder(&workspace.config().embeddings, &workspace.paths().models)
        .context("cannot index with embeddings enabled")?;
    let mut index = Index::open(&workspace, embedder, Access::ReadWrite).await?;

    let bar: Mutex<Option<ProgressBar>> = Mutex::new(Some(ui::spinner("Scanning workspace")));
    let progress = |event: Progress| {
        let mut bar = bar.lock().unwrap_or_else(|e| e.into_inner());
        match event {
            Progress::Scanning => {}
            Progress::Partition { id, files } => {
                if let Some(old) = bar.take() {
                    old.finish_and_clear();
                }
                *bar = Some(ui::counter(files as u64, id));
            }
            Progress::Files { done, .. } => {
                if let Some(bar) = bar.as_ref() {
                    bar.set_position(done as u64);
                }
            }
        }
    };
    let report = index.index_all(args.rebuild, &progress).await?;
    if let Some(bar) = bar.into_inner().unwrap_or_else(|e| e.into_inner()) {
        bar.finish_and_clear();
    }

    ui::success(&format!(
        "Indexed {} files ({} chunks), removed {}, scanned {} in {:.1}s",
        report.files_indexed,
        report.chunks,
        report.files_removed,
        report.files_scanned,
        report.duration.as_secs_f32()
    ));
    if index.embedder().is_none() {
        ui::hint("semantic search is off (embeddings disabled); results use BM25 text search");
    }
    Ok(())
}
