//! `brok`: the Brokspace command line.

mod cmd;
mod daemon;
mod integrations;
mod local_llm;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// Local-first knowledge index and event-driven agents for your workspace.
#[derive(Debug, Parser)]
#[command(name = "brok", version, about, long_about = None, propagate_version = true)]
struct Cli {
    /// Run as if brok was started in this directory.
    #[arg(short = 'C', long, global = true, value_name = "DIR")]
    dir: Option<PathBuf>,

    /// More logging (-v info, -vv debug, -vvv trace). RUST_LOG overrides.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Only print errors.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Set up Brokspace here: workspace, agent, search model, local LLM, index, coding agents.
    Init(cmd::init::Args),
    /// Index the workspace (incremental; only changed files are processed).
    Index(cmd::index::Args),
    /// Hybrid (BM25 + semantic) search over the index.
    Search(cmd::search::Args),
    /// Ask a question about the workspace (starts brok in the background if needed).
    Ask(cmd::ask::Args),
    /// Chat about the workspace, with follow-up questions.
    Chat(cmd::chat::Args),
    /// AI code review of your changes (uncommitted, staged or a branch).
    Review(cmd::git_assist::ReviewArgs),
    /// Write a commit message for the staged changes, then commit.
    Commit(cmd::git_assist::CommitArgs),
    /// Start brok in the background (bus, live indexing, agent).
    Start,
    /// Stop the background brok.
    Stop,
    /// Restart the background brok (e.g. after changing brok.yml).
    Restart,
    /// Show the background brok's log.
    Logs(cmd::service::LogsArgs),
    /// Run brok in the foreground (bus, index service, shards); `brok start` runs it in the background.
    Run(cmd::run::Args),
    /// Show workspace, index, daemon and shard status.
    Status(cmd::status::Args),
    /// Configure the local LLM, coding agent integrations or index mirroring.
    Setup(cmd::setup::Args),
    /// Check the installation and explain how to fix what is missing.
    Doctor(cmd::doctor::Args),
    /// Agent skills: list, run on the local LLM, create, export to coding agents.
    #[command(subcommand)]
    Skill(cmd::skill::Command),
    /// Serve the Brokspace tools to coding agents over MCP (stdio).
    Mcp(cmd::mcp::Args),
    /// Print shell completions (bash, zsh, fish, powershell, elvish).
    Completions {
        /// Target shell.
        shell: clap_complete::Shell,
    },
    /// Manage shards.
    #[command(subcommand)]
    Shard(cmd::shard::Command),
    /// Manage the local embedding model.
    #[command(subcommand)]
    Model(cmd::model::Command),
    /// Inspect and drive the event bus (debugging and shard development).
    #[command(subcommand)]
    Bus(cmd::bus::Command),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(&cli);

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            ui::error(&format!("cannot start the async runtime: {e}"));
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            ui::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let cwd = match cli.dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    match cli.command {
        Command::Init(args) => cmd::init::run(&cwd, args).await,
        Command::Index(args) => cmd::index::run(&cwd, args).await,
        Command::Search(args) => cmd::search::run(&cwd, args).await,
        Command::Ask(args) => cmd::ask::run(&cwd, args).await,
        Command::Chat(args) => cmd::chat::run(&cwd, args).await,
        Command::Review(args) => cmd::git_assist::review(&cwd, args).await,
        Command::Commit(args) => cmd::git_assist::commit(&cwd, args).await,
        Command::Start => cmd::service::start(&cwd).await,
        Command::Stop => cmd::service::stop(&cwd).await,
        Command::Restart => cmd::service::restart(&cwd).await,
        Command::Logs(args) => cmd::service::logs(&cwd, args).await,
        Command::Run(args) => cmd::run::run(&cwd, args).await,
        Command::Status(args) => cmd::status::run(&cwd, args).await,
        Command::Setup(args) => cmd::setup::run(&cwd, args).await,
        Command::Doctor(args) => cmd::doctor::run(&cwd, args).await,
        Command::Skill(command) => cmd::skill::run(&cwd, command).await,
        Command::Mcp(args) => cmd::mcp::run(&cwd, args).await,
        Command::Completions { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "brok", &mut std::io::stdout());
            Ok(())
        }
        Command::Shard(command) => cmd::shard::run(&cwd, command).await,
        Command::Model(command) => cmd::model::run(&cwd, command).await,
        Command::Bus(command) => cmd::bus::run(&cwd, command).await,
    }
}

fn init_tracing(cli: &Cli) {
    // `brok run` is a long-running service: show what it does by default.
    let base = match (&cli.command, cli.quiet, cli.verbose) {
        (_, true, _) => "error",
        (Command::Run(_), false, 0) => "info",
        (_, false, 0) => "warn",
        (_, false, 1) => "info",
        (_, false, 2) => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // Dependencies stay quiet unless asked for explicitly.
        EnvFilter::new(format!("warn,brok={base},brok_cli={base},brok_core={base},brok_bus={base},brok_index={base},brok_shard={base},brok_skills={base},brok_sdk={base},shard=info"))
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_target(false)
        .init();
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        super::Cli::command().debug_assert();
    }
}
