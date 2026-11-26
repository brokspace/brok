//! `brok storage`: what the workspace costs on disk (read only).

use std::path::Path;

use anyhow::Result;
use brok_index::storage::{StorageReport, report};
use console::style;
use indicatif::HumanBytes;

use crate::ui;

/// Arguments of `brok storage`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let report = tokio::task::spawn_blocking(move || report(&workspace)).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print(&report);
    }
    Ok(())
}

fn print(report: &StorageReport) {
    ui::header(&format!("This workspace: {}", HumanBytes(report.total_bytes)));
    for category in &report.categories {
        println!(
            "  {:<18} {:>11}  {}",
            category.name,
            HumanBytes(category.bytes).to_string(),
            style(&category.path).dim()
        );
    }
    if !report.partitions.is_empty() {
        println!();
        ui::header("Index by partition");
        for p in &report.partitions {
            let vectors = p
                .vector_bytes_estimate
                .map(|bytes| format!(", about {} of vectors", HumanBytes(bytes)))
                .unwrap_or_default();
            let orphaned = if p.orphaned { " (not in the workspace now)" } else { "" };
            println!("  {:<18} {:>11}  {} chunks{orphaned}", p.id, HumanBytes(p.bytes).to_string(), p.chunks);
            ui::hint(&format!(
                "  chunks {}{vectors}; keywords {}; manifest {}",
                HumanBytes(p.store_bytes),
                HumanBytes(p.keyword_bytes),
                HumanBytes(p.manifest_bytes)
            ));
        }
    }
    if let Some(models) = &report.shared_models {
        println!();
        println!("Embedding models shared by all workspaces: {}", HumanBytes(models.bytes));
        ui::hint(&models.path.display().to_string());
    }
    println!();
    if report.leftovers.is_empty() {
        ui::success("Nothing left over");
        return;
    }
    ui::header(&format!("Left over: {}", HumanBytes(report.leftover_bytes)));
    for leftover in &report.leftovers {
        println!("  {:>11}  {}", HumanBytes(leftover.bytes).to_string(), leftover.path);
        ui::hint(&format!("  {}", leftover.reason));
    }
    ui::hint("brok deletes nothing here. Check a path before removing it, with brok stopped.");
}
