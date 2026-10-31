//! `brok ask`

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use brok_sdk::payload::{AskChunk, AskDone, AskFailed, AskRequest};
use brok_sdk::{BrokClient, Event};
use console::style;
use serde_json::json;

use crate::{daemon, ui};

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
    /// Earlier turns of the conversation: a JSON file holding
    /// `[{"role": "user" | "assistant", "content": "..."}]`.
    #[arg(long, value_name = "FILE")]
    history: Option<std::path::PathBuf>,
    /// Stream JSON events, one per line, instead of text (for editors and scripts).
    #[arg(long)]
    json: bool,
}

/// Time to wait for the first answer fragment: a 7B model on a laptop CPU
/// reads a long prompt (a review, a skill) at ~15 tokens/s, so minutes can
/// pass before it writes anything.
const FIRST_TOKEN_TIMEOUT: Duration = Duration::from_secs(600);
/// Time without a new fragment once the answer is streaming.
const IDLE_TIMEOUT: Duration = Duration::from_secs(180);

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let history = match &args.history {
        Some(path) => {
            let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            serde_json::from_str(&text).with_context(|| format!("{}: expected [{{role, content}}]", path.display()))?
        }
        None => Vec::new(),
    };
    let (client, _inbox) = daemon::connect_or_start(&workspace).await?;
    let request = AskRequest {
        prompt: args.prompt.join(" "),
        provider: args.provider,
        context_chunks: args.context,
        skill: None,
        history,
    };
    answer(&client, &request, Output::text_or_json(args.json)).await.map(drop)
}

/// Where answer fragments go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Printed as they arrive; sources go to stderr.
    Terminal,
    /// JSON events on stdout, one per line: `{"type":"chunk","text":...}`
    /// while the answer streams, then `{"type":"done","provider":...,
    /// "model":...,"sources":[...]}`. Failures exit non-zero, message on stderr.
    Json,
    /// Only collected.
    Silent,
}

impl Output {
    /// `Json` when asked for, `Terminal` otherwise.
    pub fn text_or_json(json: bool) -> Self {
        if json { Output::Json } else { Output::Terminal }
    }
}

/// Sends `request` to the agent and streams the answer to stdout (sources
/// go to stderr so the answer can be piped). Returns the full answer.
pub async fn stream(client: &BrokClient, request: &AskRequest) -> Result<String> {
    answer(client, request, Output::Terminal).await
}

/// Sends `request` and returns the full answer text.
pub async fn answer(client: &BrokClient, request: &AskRequest, output: Output) -> Result<String> {
    let mut waiting = (output == Output::Terminal && ui::interactive())
        .then(|| ui::spinner("Thinking (a local model on a CPU can take a minute or more)"));
    let result = receive(client, request, output, &mut waiting).await;
    if let Some(spinner) = waiting {
        spinner.finish_and_clear();
    }
    result
}

async fn receive(
    client: &BrokClient,
    request: &AskRequest,
    output: Output,
    waiting: &mut Option<indicatif::ProgressBar>,
) -> Result<String> {
    let mut replies = client.request(Event::from_payload(request)?).await?;
    let mut stdout = std::io::stdout().lock();
    let mut text = String::new();
    loop {
        let timeout = if text.is_empty() { FIRST_TOKEN_TIMEOUT } else { IDLE_TIMEOUT };
        let reply = match replies.next_timeout(timeout).await {
            Ok(reply) => reply,
            Err(brok_sdk::ClientError::Timeout) if text.is_empty() => bail!(
                "no answer from the agent. Check `brok doctor`: the LLM must be reachable and brok-agent installed"
            ),
            Err(e) => return Err(e.into()),
        };
        if let Ok(chunk) = reply.decode::<AskChunk>() {
            if let Some(spinner) = waiting.take() {
                spinner.finish_and_clear();
            }
            match output {
                Output::Terminal => stdout.write_all(chunk.text.as_bytes())?,
                Output::Json => writeln!(stdout, "{}", json!({ "type": "chunk", "text": chunk.text }))?,
                Output::Silent => {}
            }
            stdout.flush()?;
            text.push_str(&chunk.text);
        } else if let Ok(done) = reply.decode::<AskDone>() {
            match output {
                Output::Terminal => {
                    writeln!(stdout)?;
                    print_sources(&done);
                }
                Output::Json => {
                    let event = json!({
                        "type": "done", "provider": done.provider, "model": done.model, "sources": done.sources,
                    });
                    writeln!(stdout, "{event}")?;
                    stdout.flush()?;
                }
                Output::Silent => {}
            }
            return Ok(text);
        } else if let Ok(failed) = reply.decode::<AskFailed>() {
            bail!("the agent failed: {}", failed.message);
        }
    }
}

fn print_sources(done: &AskDone) {
    if done.sources.is_empty() {
        return;
    }
    eprintln!("\n{}", style(format!("Sources ({} via {}):", done.model, done.provider)).dim());
    for source in &done.sources {
        let symbol = source.symbol.as_deref().map(|s| format!(" ({s})")).unwrap_or_default();
        eprintln!("  {}", style(format!("{}:{}-{}{symbol}", source.path, source.start_line, source.end_line)).dim());
    }
}
