//! `brok setup`: configure the local LLM, coding agent integrations and
//! index mirroring. Bare `brok setup` asks which one.

pub mod agents;
pub mod llm;
pub mod sync;

use std::path::Path;

use anyhow::{Result, bail};
use dialoguer::Select;

use crate::ui;

/// Arguments of `brok setup`.
#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Detect Ollama / LM Studio and pick a model that fits this machine.
    Llm(llm::Args),
    /// Connect coding agents (Claude Code, Cursor, VS Code, Codex): skills + MCP.
    Agents(agents::Args),
    /// Mirror the index to S3, Cloudflare R2, MinIO or Git LFS.
    Sync(sync::Args),
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let command = match args.command {
        Some(command) => command,
        None if ui::interactive() => {
            let choices = [
                "Local LLM (Ollama / LM Studio)",
                "Coding agents (Claude Code, Cursor, VS Code, Codex)",
                "Index mirroring (S3, R2, MinIO, Git LFS)",
            ];
            match Select::new().with_prompt("What do you want to set up?").items(choices).default(0).interact()? {
                0 => Command::Llm(llm::Args::default()),
                1 => Command::Agents(agents::Args::default()),
                _ => Command::Sync(sync::Args::default()),
            }
        }
        None => bail!("pick what to set up: `brok setup llm`, `brok setup agents` or `brok setup sync`"),
    };
    match command {
        Command::Llm(args) => llm::run(cwd, args).await,
        Command::Agents(args) => agents::run(cwd, args).await,
        Command::Sync(args) => sync::run(cwd, args).await,
    }
}
