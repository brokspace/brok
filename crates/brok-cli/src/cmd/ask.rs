//! `brok ask`

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use brok_sdk::Event;
use brok_sdk::payload::{AskChunk, AskDone, AskFailed, AskRequest};
use console::style;

use crate::daemon;

/// Arguments of `brok ask`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The question.
    #[arg(required = true, num_args = 1..)]
    prompt: Vec<String>,
    /// LLM provider from `llm.providers` in brok.yml.
    #[arg(short, long)]
    provider: Option<String>,
    /// Number of context chunks to retrieve.
    #[arg(short, long)]
    context: Option<usize>,
}

/// Time without any answer fragment before giving up.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let (client, _inbox) = daemon::connect(&workspace).await?;
    let request = AskRequest {
        prompt: args.prompt.join(" "),
        provider: args.provider,
        context_chunks: args.context,
        skill: None,
    };
    stream(&client, &request).await
}

/// Sends `request` to the agent and streams the answer to stdout (sources
/// go to stderr so the answer can be piped).
pub async fn stream(client: &brok_sdk::BrokClient, request: &AskRequest) -> Result<()> {
    let mut replies = client.request(Event::from_payload(request)?).await?;

    let mut stdout = std::io::stdout().lock();
    let mut answered = false;
    loop {
        let reply = match replies.next_timeout(IDLE_TIMEOUT).await {
            Ok(reply) => reply,
            Err(brok_sdk::ClientError::Timeout) if !answered => {
                bail!("no answer from the agent; is the `brok-agent` shard installed and enabled? (see `brok status`)")
            }
            Err(e) => return Err(e.into()),
        };
        if let Ok(chunk) = reply.decode::<AskChunk>() {
            answered = true;
            stdout.write_all(chunk.text.as_bytes())?;
            stdout.flush()?;
        } else if let Ok(done) = reply.decode::<AskDone>() {
            writeln!(stdout)?;
            if !done.sources.is_empty() {
                eprintln!("\n{}", style(format!("Sources ({} via {}):", done.model, done.provider)).dim());
                for source in &done.sources {
                    let symbol = source.symbol.as_deref().map(|s| format!(" ({s})")).unwrap_or_default();
                    eprintln!(
                        "  {}",
                        style(format!("{}:{}-{}{symbol}", source.path, source.start_line, source.end_line)).dim()
                    );
                }
            }
            return Ok(());
        } else if let Ok(failed) = reply.decode::<AskFailed>() {
            bail!("the agent failed: {}", failed.message);
        }
    }
}
