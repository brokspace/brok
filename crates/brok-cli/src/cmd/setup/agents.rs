//! `brok setup agents`: make coding agents Brokspace-aware.
//!
//! Two things per agent: the built-in (and workspace/shard) skills in the
//! folder the agent reads, and the `brok mcp` server so the agent can call
//! `brok_search` and `brok_status` as tools.

use std::path::Path;

use anyhow::Result;
use brok_core::Workspace;
use brok_skills::Catalog;
use dialoguer::MultiSelect;

use crate::integrations::{self, Agent, McpSetup};
use crate::ui;

/// Arguments of `brok setup agents`.
#[derive(Debug, Default, clap::Args)]
pub struct Args {
    /// Agents to configure (default: the ones detected).
    #[arg(short, long, value_enum)]
    pub agent: Vec<Agent>,
    /// Do not export skills.
    #[arg(long)]
    pub no_skills: bool,
    /// Do not register the MCP server.
    #[arg(long)]
    pub no_mcp: bool,
    /// Accept the detected agents without asking.
    #[arg(short, long)]
    pub yes: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = crate::cmd::workspace(cwd)?;
    configure(&workspace, &args)
}

/// The reusable flow, also driven by `brok init`.
pub fn configure(workspace: &Workspace, args: &Args) -> Result<()> {
    let root = workspace.root();
    let agents = if !args.agent.is_empty() {
        args.agent.clone()
    } else if ui::interactive() && !args.yes {
        let detected = integrations::detect(root);
        let labels: Vec<&str> = Agent::ALL.iter().map(|a| a.label()).collect();
        let checked: Vec<bool> = Agent::ALL.iter().map(|a| detected.contains(a)).collect();
        let picked = MultiSelect::new()
            .with_prompt("Connect which coding agents? (space toggles, enter confirms)")
            .items(&labels)
            .defaults(&checked)
            .interact()?;
        picked.into_iter().map(|i| Agent::ALL[i]).collect()
    } else {
        integrations::detect(root)
    };
    if agents.is_empty() {
        ui::step("No agent selected; nothing to do.");
        return Ok(());
    }

    if !args.no_skills {
        let catalog = Catalog::load(workspace);
        let dirs: Vec<_> = agents.iter().map(|a| a.skills_dir(root, false)).collect();
        for (dir, count) in integrations::export_skills(&catalog, &dirs)? {
            let shown =
                brok_core::fsutil::to_workspace_relative(root, &dir).unwrap_or_else(|| dir.display().to_string());
            ui::success(&format!("{count} skills in {shown}"));
        }
    }
    if !args.no_mcp {
        for agent in &agents {
            match integrations::configure_mcp(root, *agent)? {
                Some(McpSetup::Written(path)) => {
                    let shown = brok_core::fsutil::to_workspace_relative(root, &path)
                        .unwrap_or_else(|| path.display().to_string());
                    ui::success(&format!("{}: MCP server registered in {shown}", agent.label()));
                }
                Some(McpSetup::Manual { file, snippet }) => {
                    ui::step(&format!("{}: add this to {file} to enable the brok MCP tools:", agent.label()));
                    for line in snippet.lines() {
                        ui::hint(line);
                    }
                }
                None => {}
            }
        }
        if !integrations::on_path("brok") {
            ui::warn("`brok` is not on PATH, so agents cannot start `brok mcp` yet; add its folder to PATH");
        }
    }
    ui::hint(
        "In your agent, try /ask-brok or ask it to use brok_search. Commit these files to share them with your team.",
    );
    Ok(())
}
