//! `brok bus`: watch and inject events (debugging, shard development).

use std::path::Path;

use anyhow::{Context, Result};
use brok_sdk::{Event, Topic, TopicPattern};
use console::style;

use crate::{daemon, ui};

/// `brok bus` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Print events as they flow (default: every topic).
    Tail {
        /// Topic patterns, e.g. `file:*` or `agent:ask.*`.
        #[arg(default_value = "*")]
        patterns: Vec<String>,
        /// Print one JSON document per line.
        #[arg(long)]
        json: bool,
    },
    /// Publish an event as the operator.
    Emit {
        /// Topic, e.g. `file:changed`.
        topic: String,
        /// JSON payload (plain text is sent as a JSON string).
        #[arg(default_value = "null")]
        payload: String,
    },
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let (client, mut inbox) = daemon::connect(&workspace).await?;
    match command {
        Command::Tail { patterns, json } => {
            let patterns = patterns
                .into_iter()
                .map(TopicPattern::new)
                .collect::<Result<Vec<_>, _>>()
                .context("invalid pattern")?;
            client.subscribe(patterns).await?;
            if !json {
                ui::step("Listening… (Ctrl+C to stop)");
            }
            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    event = inbox.recv() => {
                        let Some(event) = event else { break };
                        if json {
                            println!("{}", serde_json::to_string(&event)?);
                        } else {
                            println!(
                                "{} {} {} {}",
                                style(event.timestamp_ms).dim(),
                                style(&event.topic).cyan().bold(),
                                style(format!("from {}", event.source)).dim(),
                                event.payload
                            );
                        }
                    }
                }
            }
        }
        Command::Emit { topic, payload } => {
            let payload = serde_json::from_str(&payload).unwrap_or(serde_json::Value::String(payload));
            let event = Event::new(Topic::new(topic)?, payload);
            let id = event.id;
            // Publishing as a request lets the bus report a rejection to us.
            let mut replies = client.request(event).await?;
            match replies.next_timeout(std::time::Duration::from_millis(300)).await {
                Err(brok_sdk::ClientError::Rejected { message, .. }) => anyhow::bail!("rejected by the bus: {message}"),
                Err(brok_sdk::ClientError::Timeout) | Ok(_) => ui::success(&format!("Published {id}")),
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(())
}
