//! `brok chat`: a conversation about the workspace, with follow-ups.

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use brok_sdk::payload::{AskRequest, ChatRole, ChatTurn, SkillInvocation};
use brok_skills::Catalog;
use console::style;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{daemon, ui};

/// Turns kept as context (each question and answer is one turn).
const MAX_TURNS: usize = 12;

/// Arguments of `brok chat`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// LLM provider from `llm.providers` in brok.yml.
    #[arg(short, long)]
    provider: Option<String>,
    /// Apply a skill to the whole conversation (e.g. `grill-me`).
    #[arg(short, long)]
    skill: Option<String>,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let catalog = Catalog::load(&workspace);
    let (client, _inbox) = daemon::connect_or_start(&workspace).await?;
    let mut skill = args.skill.as_deref().map(|name| load_skill(&catalog, name)).transpose()?;
    let mut history: Vec<ChatTurn> = Vec::new();

    ui::header("brok chat");
    ui::hint("Ask about your code; follow-ups keep the context. /help for commands, /exit to leave.");
    if let Some(skill) = &skill {
        ui::hint(&format!("skill: {}", skill.name));
    }

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        print!("\n{} ", style("›").cyan().bold());
        std::io::stdout().flush()?;
        let Some(line) = lines.next_line().await? else { break };
        let line = line.trim();
        match line {
            "" => continue,
            "/exit" | "/quit" => break,
            "/help" => {
                ui::hint("/clear          forget the conversation");
                ui::hint("/skill <name>   apply a skill (/skill off to stop); `brok skill list` shows them");
                ui::hint("/exit           leave");
                continue;
            }
            "/clear" => {
                history.clear();
                ui::step("conversation cleared");
                continue;
            }
            _ if line.starts_with("/skill") => {
                let name = line.trim_start_matches("/skill").trim();
                if name.is_empty() || name == "off" {
                    skill = None;
                    ui::step("no skill");
                } else {
                    match load_skill(&catalog, name) {
                        Ok(loaded) => {
                            ui::step(&format!("skill: {}", loaded.name));
                            skill = Some(loaded);
                        }
                        Err(e) => ui::warn(&format!("{e:#}")),
                    }
                }
                continue;
            }
            _ => {}
        }

        let request = AskRequest {
            prompt: line.to_owned(),
            provider: args.provider.clone(),
            context_chunks: None,
            skill: skill.clone(),
            history: history.clone(),
        };
        println!();
        match super::ask::stream(&client, &request).await {
            Ok(answer) => {
                history.push(ChatTurn { role: ChatRole::User, content: line.to_owned() });
                history.push(ChatTurn { role: ChatRole::Assistant, content: answer });
                let excess = history.len().saturating_sub(MAX_TURNS);
                history.drain(..excess);
            }
            Err(e) => ui::warn(&format!("{e:#}")),
        }
    }
    Ok(())
}

fn load_skill(catalog: &Catalog, name: &str) -> Result<SkillInvocation> {
    let skill = catalog.require(name)?;
    Ok(SkillInvocation { name: skill.name.clone(), instructions: skill.instructions_with_references()? })
}
