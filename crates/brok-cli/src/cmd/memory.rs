//! `brok memory`: what Brokspace remembers between sessions.
//!
//! Memories are Markdown files (see `brok_core::memory`): private ones in
//! `.brokspace/memory/`, shared ones in `.brokspace-memory/` (commit it) and
//! global ones in the user configuration directory. Searches (`brok search`,
//! `brok ask`, editor extensions, coding agents over MCP) include them.

use std::io::Read;
use std::path::Path;

use anyhow::{Result, bail};
use brok_core::memory::{Kind, Memory, NewMemory, Scope};
use brok_core::{MemoryDirs, Workspace};
use clap::Subcommand;
use console::style;
use serde_json::json;

use crate::ui;

/// `brok memory` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Remember something: a decision, a fact, a note.
    Add(AddArgs),
    /// List memories, most recent first.
    List(ListArgs),
    /// Find memories (this workspace's, your global ones and linked workspaces').
    Search(SearchArgs),
    /// Print a memory.
    Show(IdArgs),
    /// Forget a memory (private and global ones are moved to .forgotten/).
    Forget(IdArgs),
}

/// Arguments of `brok memory add`.
#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// One line summary.
    #[arg(required = true, num_args = 1..)]
    title: Vec<String>,
    /// The details (Markdown).
    #[arg(short, long, conflicts_with = "stdin")]
    body: Option<String>,
    /// Read the details from stdin.
    #[arg(long)]
    stdin: bool,
    /// note, decision, fact, session or handoff.
    #[arg(short, long, default_value = "note")]
    kind: Kind,
    /// Labels (repeat or separate with commas).
    #[arg(short, long = "tag", value_delimiter = ',')]
    tags: Vec<String>,
    /// private (this workspace), shared (committed in .brokspace-memory/) or global (every workspace).
    #[arg(short, long, default_value = "private")]
    scope: Scope,
    /// Replace the memory with this id instead of adding one.
    #[arg(long)]
    id: Option<String>,
    /// Who writes it (`ide`, `mcp`, ...).
    #[arg(long, default_value = "cli", hide = true)]
    source: String,
    /// Print the memory as JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok memory list`.
#[derive(Debug, clap::Args)]
pub struct ListArgs {
    /// Only this kind.
    #[arg(short, long)]
    kind: Option<Kind>,
    /// Only this scope.
    #[arg(short, long)]
    scope: Option<Scope>,
    /// At most this many.
    #[arg(short = 'n', long)]
    limit: Option<usize>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok memory search`.
#[derive(Debug, clap::Args)]
pub struct SearchArgs {
    /// What to look for.
    #[arg(required = true, num_args = 1..)]
    query: Vec<String>,
    /// Number of results.
    #[arg(short = 'n', long, default_value_t = 8)]
    limit: usize,
    /// Print JSON (search hits, like `brok search --json`).
    #[arg(long)]
    json: bool,
}

/// Arguments of `brok memory show` and `forget`.
#[derive(Debug, clap::Args)]
pub struct IdArgs {
    /// Memory id (the file name without `.md`).
    id: String,
    /// Look only in this scope.
    #[arg(short, long)]
    scope: Option<Scope>,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let dirs = MemoryDirs::of(workspace.paths(), workspace.config());
    match command {
        Command::Add(args) => add(&workspace, &dirs, args),
        Command::List(args) => list(&dirs, &args),
        Command::Search(args) => search(&workspace, &args),
        Command::Show(args) => {
            let memory = find(&dirs, &args)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&memory)?);
            } else {
                print_memory(&memory);
            }
            Ok(())
        }
        Command::Forget(args) => {
            let memory = dirs.forget(&args.id, args.scope)?;
            if args.json {
                println!("{}", json!({ "forgotten": memory.id, "scope": memory.scope }));
            } else {
                ui::success(&format!("Forgot `{}` ({})", memory.title, memory.scope.as_str()));
            }
            Ok(())
        }
    }
}

fn add(workspace: &Workspace, dirs: &MemoryDirs, args: AddArgs) -> Result<()> {
    let config = &workspace.config().memory;
    if !config.enabled {
        bail!("memory is off in this workspace (memory.enabled in brok.yml)");
    }
    if args.kind == Kind::Session && !config.sessions {
        // Not an error: editors save sessions unless told not to.
        if args.json {
            println!("{}", json!({ "saved": false, "reason": "memory.sessions is off" }));
        } else {
            ui::step("not saved: session memories are off (memory.sessions in brok.yml)");
        }
        return Ok(());
    }
    let body = match (args.body, args.stdin) {
        (Some(body), _) => body,
        (None, true) => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        }
        (None, false) => String::new(),
    };
    let memory = dirs.save(
        args.scope,
        NewMemory {
            id: args.id,
            title: args.title.join(" "),
            body,
            kind: args.kind,
            tags: args.tags,
            source: Some(args.source),
        },
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&memory)?);
    } else {
        ui::success(&format!("Remembered `{}` ({}, {})", memory.title, memory.kind.as_str(), memory.scope.as_str()));
        ui::hint(&memory.path.display().to_string());
        if memory.scope == Scope::Shared {
            ui::hint("shared memories are files in .brokspace-memory/: commit them to share them");
        }
    }
    Ok(())
}

fn list(dirs: &MemoryDirs, args: &ListArgs) -> Result<()> {
    let memories: Vec<Memory> = dirs
        .list()
        .into_iter()
        .filter(|m| args.kind.is_none_or(|k| k == m.kind) && args.scope.is_none_or(|s| s == m.scope))
        .take(args.limit.unwrap_or(usize::MAX))
        .collect();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&memories)?);
        return Ok(());
    }
    if memories.is_empty() {
        ui::step("no memories yet: `brok memory add \"...\"`, or ask your coding agent to remember something");
        return Ok(());
    }
    for memory in &memories {
        println!(
            "{} {} {} {}",
            style(memory.updated.get(..10).unwrap_or_default()).dim(),
            style(format!("{:<8}", memory.kind.as_str())).magenta(),
            style(&memory.title).bold(),
            style(format!("({}, {})", memory.scope.as_str(), memory.id)).dim()
        );
    }
    Ok(())
}

fn search(workspace: &Workspace, args: &SearchArgs) -> Result<()> {
    let hits = brok_index::search_memories(workspace, &args.query.join(" "), args.limit.max(1));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
        return Ok(());
    }
    if hits.is_empty() {
        ui::step("no matching memories");
        return Ok(());
    }
    for hit in &hits {
        println!(
            "{} {} {}",
            style(format!("{:.2}", hit.score)).dim(),
            style(hit.symbol.as_deref().unwrap_or_default()).bold(),
            style(format!("({})", hit.signature.as_deref().unwrap_or_default())).dim()
        );
        println!("    {}", style(&hit.path).cyan());
    }
    Ok(())
}

fn find(dirs: &MemoryDirs, args: &IdArgs) -> Result<Memory> {
    dirs.find(&args.id, args.scope).ok_or_else(|| anyhow::anyhow!("no memory `{}` (see `brok memory list`)", args.id))
}

fn print_memory(memory: &Memory) {
    println!("{}", style(&memory.title).bold());
    let mut meta = vec![memory.kind.as_str().to_owned(), memory.scope.as_str().to_owned(), memory.updated.clone()];
    if !memory.tags.is_empty() {
        meta.push(format!("#{}", memory.tags.join(" #")));
    }
    println!("{}", style(meta.join(" · ")).dim());
    if !memory.body.is_empty() {
        println!("\n{}", memory.body);
    }
    println!("\n{}", style(memory.path.display()).dim());
}
