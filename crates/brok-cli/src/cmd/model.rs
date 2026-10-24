//! `brok model`

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use brok_core::Workspace;
use brok_index::ModelFiles;
use indicatif::{MultiProgress, ProgressBar};

use crate::ui;

/// `brok model` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Download the configured embedding model.
    Download,
    /// Show where the model lives and whether it is present.
    Status,
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    match command {
        Command::Download => download(&workspace).await,
        Command::Status => {
            let files = ModelFiles::locate(&workspace.paths().models, &workspace.config().embeddings)?;
            let state = if files.is_present() { "present" } else { "missing (run `brok model download`)" };
            println!("model:     {}", workspace.config().embeddings.model);
            println!("directory: {}", files.dir.display());
            println!("status:    {state}");
            println!("backends:  {}", brok_index::compiled_features().join(", "));
            Ok(())
        }
    }
}

/// Downloads the model with one progress bar per file.
pub async fn download(workspace: &Workspace) -> Result<()> {
    let files = ModelFiles::locate(&workspace.paths().models, &workspace.config().embeddings)?;
    if files.is_present() {
        ui::success(&format!("Embedding model ready in {}", files.dir.display()));
        return Ok(());
    }
    ui::step(&format!("Downloading {} (one time)", workspace.config().embeddings.model));
    let multi = MultiProgress::new();
    let bars: Mutex<HashMap<String, ProgressBar>> = Mutex::default();
    files
        .download(&|file, done, total| {
            let mut bars = bars.lock().unwrap_or_else(|e| e.into_inner());
            let bar = bars.entry(file.to_owned()).or_insert_with(|| multi.add(ui::bytes(total, file.to_owned())));
            bar.set_position(done);
        })
        .await?;
    for bar in bars.into_inner().unwrap_or_default().into_values() {
        bar.finish();
    }
    ui::success(&format!("Embedding model ready in {}", files.dir.display()));
    Ok(())
}
