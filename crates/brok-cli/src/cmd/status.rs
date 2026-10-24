//! `brok status`

use std::path::Path;

use anyhow::Result;
use brok_index::{Access, Index};
use brok_shard::Registry;
use console::style;

use crate::{daemon, ui};

/// Arguments of `brok status`.
#[derive(Debug, clap::Args)]
pub struct Args {}

pub async fn run(cwd: &Path, _args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let config = workspace.config();
    let running = daemon::is_running(&workspace).await;

    ui::header("Workspace");
    println!("  root:        {}", workspace.root().display());
    println!(
        "  daemon:      {}",
        if running { style("running").green().to_string() } else { style("stopped").dim().to_string() }
    );
    let features = brok_index::compiled_features();
    println!("  backends:    {}", if features.is_empty() { "none (lean build)".into() } else { features.join(", ") });
    println!(
        "  embeddings:  {}",
        if config.embeddings.enabled {
            format!(
                "{} ({} dims, {:?})",
                config.embeddings.model, config.embeddings.dimensions, config.embeddings.variant
            )
        } else {
            "disabled (BM25 only)".into()
        }
    );
    println!("  llm:         {}", config.llm.default);
    println!("  sync:        {:?}", config.sync.backend);

    println!();
    ui::header("Index");
    let index = Index::open(&workspace, None, Access::ReadOnly).await?;
    let partitions = index.status().await?;
    if partitions.is_empty() {
        println!("  not indexed yet (run `brok index`)");
    }
    for p in partitions {
        let path = if p.path.is_empty() { "." } else { p.path.as_str() };
        println!(
            "  {:<5} {:<28} {:>6} files {:>7} chunks  {} {}",
            format!("{:?}", p.tier).to_lowercase(),
            path,
            p.files,
            p.chunks,
            p.backend,
            if p.semantic { "hybrid" } else { "bm25" }
        );
    }

    println!();
    ui::header("Shards");
    let shards = Registry::new(workspace.paths()).list()?;
    if shards.is_empty() {
        println!("  none installed");
    }
    for shard in shards {
        let enabled = config.shard_enabled(&shard.manifest.name);
        let state = match (enabled, shard.manifest.has_process(), shard.is_runnable()) {
            (false, _, _) => style("disabled").dim().to_string(),
            (true, false, _) => style("skills").cyan().to_string(),
            (true, true, false) => style("missing binary").red().to_string(),
            (true, true, true) => style("enabled").green().to_string(),
        };
        let source = shard.lock.as_ref().map(|l| l.source.to_string()).unwrap_or_default();
        println!("  {:<24} {:<10} {:<16} {}", shard.manifest.name, shard.manifest.version, state, style(source).dim());
    }
    Ok(())
}
