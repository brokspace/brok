//! `brok status`

use std::path::Path;

use anyhow::Result;
use brok_core::Workspace;
use brok_index::{Access, Index, PartitionStatus};
use brok_shard::{InstalledShard, Registry};
use console::style;
use serde_json::{Value, json};

use crate::{daemon, ui};

/// Arguments of `brok status`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Print JSON (for editors and scripts). A folder without a workspace
    /// prints `{"initialized": false}` instead of failing.
    #[arg(long)]
    json: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    if args.json {
        let status = match Workspace::discover(cwd) {
            Ok(workspace) => json_status(&workspace).await?,
            Err(_) => json!({ "version": env!("CARGO_PKG_VERSION"), "initialized": false }),
        };
        println!("{status}");
        return Ok(());
    }

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
    let partitions = partitions(&workspace).await?;
    if partitions.is_empty() {
        println!("  not indexed yet (run `brok index`)");
    }
    for p in partitions {
        println!(
            "  {:<5} {:<28} {:>6} files {:>7} chunks  {} {}",
            tier(&p),
            display_path(&p),
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
        let state = match shard_state(&workspace, &shard) {
            "disabled" => style("disabled").dim().to_string(),
            "skills" => style("skills").cyan().to_string(),
            "missing-binary" => style("missing binary").red().to_string(),
            other => style(other).green().to_string(),
        };
        let source = shard.lock.as_ref().map(|l| l.source.to_string()).unwrap_or_default();
        println!("  {:<24} {:<10} {:<16} {}", shard.manifest.name, shard.manifest.version, state, style(source).dim());
    }
    Ok(())
}

async fn json_status(workspace: &Workspace) -> Result<Value> {
    let config = workspace.config();
    let llm = config.llm.providers.get(&config.llm.default);
    let index: Vec<Value> = partitions(workspace)
        .await?
        .iter()
        .map(|p| {
            json!({
                "tier": tier(p), "path": display_path(p), "files": p.files, "chunks": p.chunks,
                "backend": p.backend, "semantic": p.semantic,
            })
        })
        .collect();
    let shards: Vec<Value> = Registry::new(workspace.paths())
        .list()?
        .iter()
        .map(|shard| {
            json!({
                "name": shard.manifest.name,
                "version": shard.manifest.version.to_string(),
                "description": shard.manifest.description,
                "state": shard_state(workspace, shard),
                "source": shard.lock.as_ref().map(|l| l.source.to_string()),
            })
        })
        .collect();
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "initialized": true,
        "root": workspace.root(),
        "daemon": { "running": daemon::is_running(workspace).await },
        "backends": brok_index::compiled_features(),
        "embeddings": {
            "enabled": config.embeddings.enabled,
            "model": config.embeddings.model,
            "dimensions": config.embeddings.dimensions,
        },
        "llm": {
            "provider": config.llm.default,
            "model": llm.map(|p| p.model.as_str()),
            "base_url": llm.map(|p| p.base_url.as_str()),
        },
        "index": index,
        "shards": shards,
    }))
}

async fn partitions(workspace: &Workspace) -> Result<Vec<PartitionStatus>> {
    let index = Index::open(workspace, None, Access::ReadOnly).await?;
    index.status().await
}

fn tier(p: &PartitionStatus) -> String {
    format!("{:?}", p.tier).to_lowercase()
}

fn display_path(p: &PartitionStatus) -> &str {
    if p.path.is_empty() { "." } else { p.path.as_str() }
}

/// `enabled`, `disabled`, `skills` (a skill-only shard) or `missing-binary`.
fn shard_state(workspace: &Workspace, shard: &InstalledShard) -> &'static str {
    match (workspace.config().shard_enabled(&shard.manifest.name), shard.manifest.has_process(), shard.is_runnable()) {
        (false, _, _) => "disabled",
        (true, false, _) => "skills",
        (true, true, false) => "missing-binary",
        (true, true, true) => "enabled",
    }
}
