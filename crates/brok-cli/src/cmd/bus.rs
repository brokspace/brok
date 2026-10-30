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
                                style(time_of_day(event.timestamp_ms)).dim(),
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

/// `HH:MM:SS.mmm` in UTC, like the timestamps of `brok logs`.
fn time_of_day(timestamp_ms: u64) -> String {
    let ms = timestamp_ms % 86_400_000;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

#[cfg(test)]
mod tests {
    #[test]
    fn time_of_day_is_utc() {
        // 2026-10-30T14:24:08.291Z
        assert_eq!(super::time_of_day(1_793_370_248_291), "14:24:08.291");
        assert_eq!(super::time_of_day(0), "00:00:00.000");
    }
}
