//! `brok workspace`: the Meta Workspace (ADR 0014). Register independent
//! repositories as members of this workspace, relate them, and see their
//! state. Opt-in: a workspace without members behaves as before, and members
//! are never written to.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use brok_core::Workspace;
use brok_core::members::{self, Member, MemberState, Registration, RelationKind, Relationship};
use brok_core::provider::default_source_control;
use clap::Subcommand;
use console::style;
use serde_json::json;

use crate::ui;

/// `brok workspace` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Register a repository (a directory on this machine) as a member.
    Add(AddArgs),
    /// List the members and their relationships.
    List(JsonArgs),
    /// Show each member's state: repository, changes, whether it has its own Brokspace.
    Status(JsonArgs),
    /// Unregister a member (its directory is never touched).
    Remove(RemoveArgs),
    /// Record that one member relates to another (`brok workspace relate web consumes_api api`).
    Relate(RelateArgs),
    /// Remove a recorded relationship.
    Unrelate(RelateArgs),
    /// List repositories next to this workspace (or in a directory) that are not members yet.
    Discover(DiscoverArgs),
}

/// Arguments of `brok workspace add`.
#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// Directory of the repository, absolute or relative to this workspace.
    path: PathBuf,
    /// Member id (default: the directory name, as lowercase kebab-case).
    #[arg(long)]
    id: Option<String>,
    /// Display name.
    #[arg(long)]
    name: Option<String>,
    /// What it is and how it relates to the rest (helps the LLM).
    #[arg(short, long)]
    description: Option<String>,
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

/// Arguments of `brok workspace remove`.
#[derive(Debug, clap::Args)]
pub struct RemoveArgs {
    /// Member id.
    id: String,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok workspace relate` and `unrelate`.
#[derive(Debug, clap::Args)]
pub struct RelateArgs {
    /// Source member id.
    from: String,
    /// depends_on, consumes_api, publishes_api, uses_library, deploys_with, tests or shares_contract_with.
    kind: String,
    /// Target member id.
    to: String,
}

/// Arguments of `brok workspace discover`.
#[derive(Debug, clap::Args)]
pub struct DiscoverArgs {
    /// Directory to look in (default: the parent of this workspace).
    dir: Option<PathBuf>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    let mut workspace = super::workspace(cwd)?;
    match command {
        Command::Add(args) => add(&mut workspace, args),
        Command::List(args) => list(&workspace, args.json),
        Command::Status(args) => status(&workspace, args.json),
        Command::Remove(args) => remove(&mut workspace, &args),
        Command::Relate(args) => relate(&mut workspace, &args, true),
        Command::Unrelate(args) => relate(&mut workspace, &args, false),
        Command::Discover(args) => discover(&workspace, &args),
    }
}

fn add(workspace: &mut Workspace, args: AddArgs) -> Result<()> {
    let mut config = workspace.config().clone();
    let registration = members::register(
        workspace.root(),
        &mut config.workspace.members,
        &args.path,
        args.id.as_deref(),
        args.name.filter(|n| !n.trim().is_empty()),
        args.description.filter(|d| !d.trim().is_empty()),
    )?;
    let (id, changed) = match &registration {
        Registration::Added { id } => (id.clone(), true),
        Registration::Existing { id } => (id.clone(), false),
    };
    // Only touch brok.yml when something changed, so repeating the command is a true no-op.
    if changed {
        workspace.save_config(config)?;
    }
    if args.json {
        println!("{}", json!({ "id": id, "added": changed }));
    } else if changed {
        ui::success(&format!("Registered `{id}`"));
    } else {
        ui::step(&format!("`{id}` is already a member"));
    }
    Ok(())
}

fn remove(workspace: &mut Workspace, args: &RemoveArgs) -> Result<()> {
    let mut config = workspace.config().clone();
    if config.workspace.members.remove(&args.id).is_none() {
        bail!("no member `{}` (see `brok workspace list`)", args.id);
    }
    config.workspace.relationships.retain(|r| r.from != args.id && r.to != args.id);
    workspace.save_config(config)?;
    if args.json {
        println!("{}", json!({ "removed": args.id }));
    } else {
        ui::success(&format!("Unregistered `{}` (its directory was not touched)", args.id));
    }
    Ok(())
}

fn relate(workspace: &mut Workspace, args: &RelateArgs, add: bool) -> Result<()> {
    let Some(kind) = RelationKind::parse(&args.kind) else {
        bail!(
            "unknown relationship `{}` (depends_on, consumes_api, publishes_api, uses_library, deploys_with, tests, shares_contract_with)",
            args.kind
        );
    };
    let edge = Relationship { from: args.from.clone(), kind, to: args.to.clone() };
    let mut config = workspace.config().clone();
    let exists = config.workspace.relationships.contains(&edge);
    if add {
        if exists {
            ui::step("already recorded");
            return Ok(());
        }
        config.workspace.relationships.push(edge);
        config.workspace.relationships.sort();
        config.validate()?; // unknown members, self edges
        workspace.save_config(config)?;
        ui::success(&format!("{} {} {}", args.from, args.kind, args.to));
    } else {
        if !exists {
            bail!("no such relationship");
        }
        config.workspace.relationships.retain(|r| *r != edge);
        workspace.save_config(config)?;
        ui::success("Removed");
    }
    Ok(())
}

fn list(workspace: &Workspace, json: bool) -> Result<()> {
    let config = &workspace.config().workspace;
    if json {
        let members: Vec<_> = Member::all(workspace)
            .iter()
            .map(|m| json!({ "id": m.id, "name": m.name, "path": m.config.path, "root": m.root, "description": m.config.description }))
            .collect();
        let relationships: Vec<_> = config
            .relationships
            .iter()
            .map(|r| json!({ "from": r.from, "kind": r.kind.as_str(), "to": r.to }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&json!({ "members": members, "relationships": relationships }))?);
        return Ok(());
    }
    if config.members.is_empty() {
        ui::step("no members: `brok workspace add <path>` (a workspace without members works as before)");
        return Ok(());
    }
    for member in Member::all(workspace) {
        println!("  {:<20} {}", style(&member.id).bold(), member.config.path);
        if let Some(description) = &member.config.description {
            println!("  {:<20} {}", "", style(description).dim());
        }
    }
    for edge in &config.relationships {
        println!("  {} {} {}", edge.from, style(edge.kind.as_str()).cyan(), edge.to);
    }
    Ok(())
}

fn status(workspace: &Workspace, json: bool) -> Result<()> {
    let provider = default_source_control();
    let mut rows = Vec::new();
    for member in Member::all(workspace) {
        let state = member.state();
        let (changes, head) = if state == MemberState::Repository {
            (
                provider.status(&member.root).ok().map(|c| c.len()),
                provider.head_revision(&member.root).ok().flatten().map(|r| r.0.chars().take(8).collect::<String>()),
            )
        } else {
            (None, None)
        };
        rows.push((member.id.clone(), member.is_workspace(), state, changes, head));
    }
    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|(id, own, state, changes, head)| {
                json!({ "id": id, "state": state, "has_own_brokspace": own, "uncommitted_changes": changes, "head": head })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if rows.is_empty() {
        ui::step("no members: `brok workspace add <path>`");
    }
    for (id, own, state, changes, head) in rows {
        let label = match state {
            MemberState::Repository => style("repository").green(),
            MemberState::Directory => style("directory").yellow(),
            MemberState::Missing => style("missing").red(),
        };
        let detail = match (changes, head) {
            (Some(n), Some(h)) => format!("{h}, {n} uncommitted change(s)"),
            (Some(n), None) => format!("no commits, {n} uncommitted change(s)"),
            _ => String::new(),
        };
        let own = if own { "own brokspace" } else { "" };
        println!("  {:<20} {:<10} {:<14} {}", style(id).bold(), label, own, detail);
    }
    Ok(())
}

fn discover(workspace: &Workspace, args: &DiscoverArgs) -> Result<()> {
    let dir = match &args.dir {
        Some(dir) if dir.is_absolute() => dir.clone(),
        Some(dir) => workspace.root().join(dir),
        None => workspace.root().parent().map(Path::to_path_buf).unwrap_or_else(|| workspace.root().to_path_buf()),
    };
    let found = members::discover(workspace.root(), &dir, &workspace.config().workspace.members);
    if args.json {
        let paths: Vec<_> = found.iter().map(|p| p.display().to_string()).collect();
        println!("{}", serde_json::to_string_pretty(&paths)?);
        return Ok(());
    }
    if found.is_empty() {
        ui::step(&format!("no unregistered repositories in {}", dir.display()));
    }
    for path in found {
        println!("  {}", path.display());
    }
    Ok(())
}
