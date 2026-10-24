//! `brok skill`

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use brok_sdk::payload::{AskRequest, SkillInvocation};
use brok_skills::{Catalog, Skill};
use console::style;

use crate::integrations::{self, Agent};
use crate::{daemon, ui};

/// `brok skill` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// List every available skill (workspace, shards and built-in).
    List {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print a skill's SKILL.md.
    Show {
        /// Skill name.
        name: String,
    },
    /// Run a skill on the local LLM through brok-agent (needs `brok run`).
    Run {
        /// Skill name.
        name: String,
        /// What to apply the skill to (a task, a question, a diff summary...).
        input: Vec<String>,
        /// LLM provider from `llm.providers` in brok.yml.
        #[arg(short, long)]
        provider: Option<String>,
    },
    /// Create a new skill in `.brokspace/skills/` (or `--dir`).
    New {
        /// Skill name (lowercase, dashes).
        name: String,
        /// Only a human can invoke it (an orchestrating workflow).
        #[arg(long)]
        user_invoked: bool,
        /// Parent directory (e.g. `skills` inside a skill shard).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Install the skills into coding agents' skill folders.
    Export {
        /// Target agents (default: detected agents plus `.agents/skills`).
        #[arg(short, long, value_enum)]
        agent: Vec<Agent>,
        /// Install into your home directory instead of this workspace.
        #[arg(long)]
        global: bool,
        /// Export into this directory instead.
        #[arg(long, conflicts_with_all = ["agent", "global"])]
        dir: Option<PathBuf>,
    },
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    // Creating a skill in an explicit directory (e.g. a skill shard) needs no workspace.
    if let Command::New { name, user_invoked, dir: Some(dir) } = &command {
        return new_skill(dir, name, *user_invoked);
    }
    let workspace = super::workspace(cwd)?;
    let catalog = Catalog::load(&workspace);
    for error in catalog.errors() {
        ui::warn(&format!("skipped skill: {error}"));
    }
    match command {
        Command::List { json } => list(&catalog, json),
        Command::Show { name } => {
            let skill = catalog.require(&name)?;
            println!("{}", style(format!("{} ({}, {})", skill.name, invocation(skill), skill.source)).dim());
            println!("{}", skill.body);
            Ok(())
        }
        Command::Run { name, input, provider } => {
            let skill = catalog.require(&name)?;
            let (client, _inbox) = daemon::connect(&workspace).await?;
            let prompt = if input.is_empty() {
                "Apply this skill to the current workspace. Start with the first step.".to_owned()
            } else {
                input.join(" ")
            };
            let request = AskRequest {
                prompt,
                provider,
                context_chunks: None,
                skill: Some(SkillInvocation {
                    name: skill.name.clone(),
                    instructions: skill.instructions_with_references()?,
                }),
            };
            super::ask::stream(&client, &request).await
        }
        Command::New { name, user_invoked, dir } => {
            let parent = dir.unwrap_or_else(|| brok_skills::workspace_skills_dir(&workspace));
            new_skill(&parent, &name, user_invoked)
        }
        Command::Export { agent, global, dir } => {
            let dirs = match dir {
                Some(dir) => vec![dir],
                None => {
                    let agents = if agent.is_empty() { integrations::detect(workspace.root()) } else { agent };
                    agents.iter().map(|a| a.skills_dir(workspace.root(), global)).collect()
                }
            };
            if catalog.is_empty() {
                bail!("no skills to export");
            }
            for (dir, count) in integrations::export_skills(&catalog, &dirs)? {
                ui::success(&format!("Exported {count} skills to {}", dir.display()));
            }
            ui::hint("Re-run after `brok` upgrades or skill changes; folders brok did not create are never touched.");
            Ok(())
        }
    }
}

fn new_skill(parent: &Path, name: &str, user_invoked: bool) -> Result<()> {
    let created = brok_skills::scaffold(parent, name, user_invoked)?;
    ui::success(&format!("Created {}", created.join(brok_skills::SKILL_FILE).display()));
    ui::hint("Edit the description first: it decides when agents reach for the skill.");
    ui::hint(&format!("Try it: brok skill run {name} \"...\"   Share it: brok skill export"));
    Ok(())
}

fn invocation(skill: &Skill) -> &'static str {
    if skill.user_invoked { "user-invoked" } else { "model-invoked" }
}

fn list(catalog: &Catalog, json: bool) -> Result<()> {
    if json {
        let rows: Vec<serde_json::Value> = catalog
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "user_invoked": s.user_invoked,
                    "source": s.source.to_string(),
                    "bucket": s.bucket,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    for (title, user_invoked) in [("User-invoked (type /name in your agent)", true), ("Model-invoked", false)] {
        ui::header(title);
        for skill in catalog.iter().filter(|s| s.user_invoked == user_invoked) {
            let mut description = skill.description.clone();
            if description.chars().count() > 90 {
                description = description.chars().take(87).collect::<String>() + "...";
            }
            println!(
                "  {:<22} {:<12} {}",
                style(&skill.name).cyan(),
                style(skill.source.to_string()).dim(),
                description
            );
        }
        println!();
    }
    for (name, source) in catalog.shadowed() {
        ui::hint(&format!("{name} from {source} is overridden by a higher-priority skill"));
    }
    Ok(())
}
