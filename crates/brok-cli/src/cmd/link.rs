//! `brok link`: workspaces this one depends on (`workspace.links`).
//!
//! A linked workspace's code and memories are searched with this one's, so
//! `brok ask`, editors and coding agents know about the services and
//! libraries the code calls.

use std::path::Path;

use anyhow::{Result, bail};
use brok_core::config::{LinkConfig, is_link_name};
use brok_core::links::{LinkIndex, LinkKind};
use brok_core::{Link, Workspace};
use brok_index::{Access, Index, LinkStatus};
use clap::Subcommand;
use console::style;
use serde_json::{Value, json};

use crate::{daemon, ui};

/// `brok link` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Link a workspace: a directory on this machine or a git repository.
    Add(AddArgs),
    /// List linked workspaces and whether they are indexed.
    List(JsonArgs),
    /// Unlink a workspace (its clone and index in .brokspace/links/ are deleted).
    Remove(RemoveArgs),
    /// Clone or pull git links and index the links brok manages.
    Sync(SyncArgs),
}

/// Arguments of `brok link add`.
#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// Name, used in results (`@checkout`) and in brok.yml.
    name: String,
    /// A directory (absolute or relative to this workspace) or a git URL.
    target: String,
    /// Branch of a git repository.
    #[arg(short, long)]
    branch: Option<String>,
    /// What it is and how this workspace uses it (helps the LLM).
    #[arg(short, long)]
    description: Option<String>,
    /// Do not search its memories.
    #[arg(long)]
    no_memory: bool,
    /// Only write brok.yml; clone and index later with `brok link sync`.
    #[arg(long)]
    no_sync: bool,
    /// Print the link as JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok link remove`.
#[derive(Debug, clap::Args)]
pub struct RemoveArgs {
    /// Name of the link.
    name: String,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok link sync`.
#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Links to sync (all when omitted).
    names: Vec<String>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// `--json` only.
#[derive(Debug, clap::Args)]
pub struct JsonArgs {
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    let mut workspace = super::workspace(cwd)?;
    match command {
        Command::Add(args) => add(&mut workspace, args).await,
        Command::List(args) => list(&workspace, args.json).await,
        Command::Remove(args) => remove(&mut workspace, &args).await,
        Command::Sync(args) => {
            let results = sync(&workspace, &args.names, !args.json).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            }
            if results.iter().any(|r| r["error"].is_string()) && !args.json {
                bail!("some links could not be synced");
            }
            Ok(())
        }
    }
}

async fn add(workspace: &mut Workspace, args: AddArgs) -> Result<()> {
    if !is_link_name(&args.name) {
        bail!("`{}` is not a valid link name (letters, digits, `-`, `_` and `.`)", args.name);
    }
    if workspace.config().workspace.links.contains_key(&args.name) {
        bail!("`{}` is already linked (`brok link remove {}` first)", args.name, args.name);
    }
    let mut link = if is_git_url(&args.target, workspace.root()) {
        LinkConfig::git(&args.target, args.branch.clone())
    } else {
        if args.branch.is_some() {
            bail!("--branch only applies to git repositories");
        }
        let dir = workspace.root().join(&args.target);
        if !dir.is_dir() {
            bail!("{} is not a directory (or a git URL)", dir.display());
        }
        if brok_core::workspace::dunce_canonicalize(&dir)? == workspace.root() {
            bail!("a workspace cannot link itself");
        }
        LinkConfig::local(args.target.replace('\\', "/"))
    };
    link.description = args.description.filter(|d| !d.trim().is_empty());
    link.memory = !args.no_memory;

    let mut config = workspace.config().clone();
    config.workspace.links.insert(args.name.clone(), link);
    workspace.save_config(config)?;
    if !args.json {
        ui::success(&format!("Linked `{}` ({})", args.name, args.target));
    }

    let mut result = json!({ "name": args.name });
    if !args.no_sync {
        let synced = sync(workspace, std::slice::from_ref(&args.name), !args.json).await?;
        result["sync"] = synced.into_iter().next().unwrap_or(Value::Null);
    }
    restart_daemon(workspace, args.json).await;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

async fn remove(workspace: &mut Workspace, args: &RemoveArgs) -> Result<()> {
    let mut config = workspace.config().clone();
    if config.workspace.links.remove(&args.name).is_none() {
        bail!("no link named `{}` (see `brok link list`)", args.name);
    }
    workspace.save_config(config)?;
    // The clone and index brok made for it (never the linked directory itself).
    let dir = workspace.paths().links.join(&args.name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| anyhow::anyhow!("removing {}: {e}", dir.display()))?;
    }
    restart_daemon(workspace, args.json).await;
    if args.json {
        println!("{}", json!({ "removed": args.name }));
    } else {
        ui::success(&format!("Unlinked `{}`", args.name));
    }
    Ok(())
}

async fn list(workspace: &Workspace, json: bool) -> Result<()> {
    let links = statuses(workspace).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&links)?);
        return Ok(());
    }
    if links.is_empty() {
        ui::step("no linked workspaces: `brok link add <name> <path or git URL>`");
        return Ok(());
    }
    for link in &links {
        let state = match &link.error {
            Some(error) => style(error.clone()).yellow().to_string(),
            None => style(format!("{} files, {} chunks", link.files, link.chunks)).green().to_string(),
        };
        let source = link.git.clone().unwrap_or_else(|| link.root.clone());
        println!("  {:<16} {:<6} {}  {}", style(&link.name).bold(), kind(link.kind), source, state);
        if let Some(description) = &link.description {
            println!("  {:<16} {}", "", style(description).dim());
        }
    }
    Ok(())
}

/// Status of every link (opens the index read only).
pub async fn statuses(workspace: &Workspace) -> Result<Vec<LinkStatus>> {
    if workspace.config().workspace.links.is_empty() {
        return Ok(Vec::new());
    }
    Ok(Index::open(workspace, None, Access::ReadOnly).await?.link_status().await)
}

/// Clones or pulls git links and indexes the ones brok manages (and linked
/// Brokspace workspaces whose own brok is not running). One JSON result per
/// link; failures are reported per link.
pub async fn sync(workspace: &Workspace, names: &[String], verbose: bool) -> Result<Vec<Value>> {
    let links = Link::all(workspace);
    for name in names {
        if !links.iter().any(|l| &l.name == name) {
            bail!("no link named `{name}` (see `brok link list`)");
        }
    }
    let mut results = Vec::new();
    for link in links.into_iter().filter(|l| names.is_empty() || names.contains(&l.name)) {
        let result = sync_one(workspace, &link, verbose).await;
        let value = match result {
            Ok(value) => value,
            Err(e) => {
                if verbose {
                    ui::warn(&format!("{}: {e:#}", link.name));
                }
                json!({ "name": link.name, "error": format!("{e:#}") })
            }
        };
        results.push(value);
    }
    Ok(results)
}

async fn sync_one(workspace: &Workspace, link: &Link, verbose: bool) -> Result<Value> {
    let mut out = json!({ "name": link.name });
    if link.kind == LinkKind::Git {
        let spinner = verbose.then(|| ui::spinner(format!("Fetching {}", link.name)));
        let fetched = tokio::task::spawn_blocking({
            let link = link.clone();
            move || link.fetch()
        })
        .await?;
        if let Some(spinner) = spinner {
            spinner.finish_and_clear();
        }
        out["commit"] = json!(fetched?);
    }
    let target = link.workspace(workspace.config())?;
    if link.index() == LinkIndex::Own && daemon::is_running(&target).await {
        if verbose {
            ui::step(&format!("{}: indexed by its own brok (running)", link.name));
        }
        out["indexed"] = json!("own");
        return Ok(out);
    }
    let (report, _) = super::index::index_workspace(&target, false, verbose).await?;
    if verbose {
        ui::success(&format!(
            "{}: {} files indexed, {} removed, {} unchanged",
            link.name, report.files_indexed, report.files_removed, report.files_unchanged
        ));
    }
    out["files_indexed"] = json!(report.files_indexed);
    out["files_removed"] = json!(report.files_removed);
    Ok(out)
}

/// The daemon reads `workspace.links` when it starts: restart it so
/// searches see the change.
async fn restart_daemon(workspace: &Workspace, quiet: bool) {
    if !daemon::is_running(workspace).await {
        return;
    }
    let restarted = async {
        daemon::stop(workspace).await?;
        daemon::start(workspace).await
    }
    .await;
    match restarted {
        Ok(_) if !quiet => ui::step("restarted the background brok"),
        Ok(_) => {}
        Err(e) => ui::warn(&format!("restart brok to use the change (`brok restart`): {e:#}")),
    }
}

/// Whether `target` names a git repository rather than a directory.
fn is_git_url(target: &str, root: &Path) -> bool {
    let url_like = target.contains("://") || target.starts_with("git@") || target.ends_with(".git");
    url_like && !root.join(target).is_dir()
}

fn kind(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Local => "local",
        LinkKind::Git => "git",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_urls() {
        let root = std::env::temp_dir();
        assert!(is_git_url("https://github.com/acme/checkout.git", &root));
        assert!(is_git_url("git@github.com:acme/checkout.git", &root));
        assert!(is_git_url("https://gitlab.com/acme/checkout", &root));
        assert!(!is_git_url("../checkout", &root));
    }
}
