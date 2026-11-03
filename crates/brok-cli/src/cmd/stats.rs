//! `brok stats`: what the local metrics say (`brok_core::metrics`).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use brok_core::metrics::{Event, percentile};
use console::style;
use serde::Serialize;

use crate::ui;

/// Arguments of `brok stats`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Period, in days.
    #[arg(short, long, default_value_t = 7)]
    days: u32,
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// Latency figures, in milliseconds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Latency {
    /// Events measured.
    pub count: usize,
    /// Median.
    pub p50_ms: Option<u64>,
    /// 95th percentile.
    pub p95_ms: Option<u64>,
}

impl Latency {
    fn of(values: &[u64]) -> Self {
        Self { count: values.len(), p50_ms: percentile(values, 50.0), p95_ms: percentile(values, 95.0) }
    }
}

/// Searches: by the CLI, the IDE, coding agents (MCP) and the agent.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Searches {
    /// Latency of every search.
    pub latency: Latency,
    /// Searches that found nothing.
    pub empty: usize,
    /// Searches that used embeddings.
    pub semantic: usize,
    /// Searches with results from linked workspaces.
    pub with_links: usize,
    /// Searches with memories among the results.
    pub with_memories: usize,
}

/// Answers: questions, reviews, commit messages and skills.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Answers {
    /// Total time of the answers that completed.
    pub latency: Latency,
    /// Time to the first words.
    pub first_token: Latency,
    /// Answers that failed (LLM unreachable, timeouts, ...).
    pub failed: usize,
    /// Completed answers per `provider/model`.
    pub models: BTreeMap<String, usize>,
    /// Answers per skill (`ask` for plain questions).
    pub skills: BTreeMap<String, usize>,
}

/// Indexing runs: `brok index`, the daemon catching up and following edits.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Indexing {
    /// Duration of the runs.
    pub latency: Latency,
    /// Files (re)indexed in total.
    pub files_indexed: u64,
    /// When the last run happened.
    pub last: Option<String>,
}

/// A failed command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    /// When.
    pub at: String,
    /// The subcommand.
    pub command: String,
    /// The message (secrets masked).
    pub message: String,
}

/// Errors in the period.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Errors {
    /// How many.
    pub count: usize,
    /// Per subcommand.
    pub commands: BTreeMap<String, usize>,
    /// The latest few, newest first.
    pub recent: Vec<Failure>,
}

/// `brok stats --json`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Period, in days.
    pub days: u32,
    /// Events read.
    pub events: usize,
    /// Searches.
    pub search: Searches,
    /// Answers.
    pub answers: Answers,
    /// Indexing.
    pub index: Indexing,
    /// Failed commands.
    pub errors: Errors,
}

/// Errors kept in `Errors::recent`.
const RECENT_ERRORS: usize = 5;

/// Sums up `events` (oldest first).
pub fn summarize(events: &[Event], days: u32) -> Summary {
    let ms = |kind: &str, filter: &dyn Fn(&Event) -> bool| -> Vec<u64> {
        events.iter().filter(|e| e.kind == kind && filter(e)).filter_map(|e| e.ms).collect()
    };
    let searches: Vec<&Event> = events.iter().filter(|e| e.kind == "search").collect();
    let count = |filter: &dyn Fn(&&Event) -> bool| searches.iter().filter(|e| filter(e)).count();
    let search = Searches {
        latency: Latency::of(&ms("search", &|_| true)),
        empty: count(&|e| e.number("hits") == Some(0.0)),
        semantic: count(&|e| e.flag("semantic") == Some(true)),
        with_links: count(&|e| e.number("links").is_some_and(|n| n > 0.0)),
        with_memories: count(&|e| e.number("memories").is_some_and(|n| n > 0.0)),
    };

    let ok = |e: &Event| e.flag("ok") != Some(false);
    let mut answers = Answers {
        latency: Latency::of(&ms("answer", &ok)),
        first_token: Latency::of(
            &events
                .iter()
                .filter(|e| e.kind == "answer")
                .filter_map(|e| e.number("first_token_ms"))
                .map(|n| n as u64)
                .collect::<Vec<_>>(),
        ),
        ..Answers::default()
    };
    for event in events.iter().filter(|e| e.kind == "answer") {
        if ok(event) {
            if let (Some(provider), Some(model)) = (event.text("provider"), event.text("model")) {
                *answers.models.entry(format!("{provider}/{model}")).or_default() += 1;
            }
        } else {
            answers.failed += 1;
        }
        *answers.skills.entry(event.text("skill").unwrap_or("ask").to_owned()).or_default() += 1;
    }

    let runs: Vec<&Event> = events.iter().filter(|e| e.kind == "index").collect();
    let index = Indexing {
        latency: Latency::of(&ms("index", &|_| true)),
        files_indexed: runs.iter().filter_map(|e| e.number("files_indexed")).map(|n| n as u64).sum(),
        last: runs.last().map(|e| e.at.clone()),
    };

    let failures: Vec<&Event> = events.iter().filter(|e| e.kind == "error").collect();
    let mut errors = Errors { count: failures.len(), ..Errors::default() };
    for failure in &failures {
        *errors.commands.entry(failure.text("command").unwrap_or("?").to_owned()).or_default() += 1;
    }
    errors.recent = failures
        .iter()
        .rev()
        .take(RECENT_ERRORS)
        .map(|e| Failure {
            at: e.at.clone(),
            command: e.text("command").unwrap_or("?").to_owned(),
            message: e.text("message").unwrap_or_default().to_owned(),
        })
        .collect();

    Summary { days, events: events.len(), search, answers, index, errors }
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let days = args.days.max(1);
    let summary = summarize(&brok_core::metrics::load(workspace.paths(), days), days);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }
    if !workspace.config().metrics.enabled {
        ui::warn("metrics are off in this workspace (metrics.enabled in brok.yml)");
    }
    print(&summary);
    Ok(())
}

fn print(summary: &Summary) {
    ui::header(&format!("Last {} day(s)", summary.days));
    if summary.events == 0 {
        println!("  nothing recorded yet");
        return;
    }
    let s = &summary.search;
    println!(
        "  {:<9} {} · {} with no results · {} used linked workspaces · {} found memories",
        style("search").bold(),
        latency(&s.latency, "search", "searches"),
        s.empty,
        s.with_links,
        s.with_memories
    );
    let a = &summary.answers;
    let models = top(&a.models);
    println!(
        "  {:<9} {} · {} failed · first words {}{}",
        style("answers").bold(),
        latency(&a.latency, "answer", "answers"),
        a.failed,
        a.first_token.p50_ms.map_or_else(|| "-".to_owned(), duration),
        if models.is_empty() { String::new() } else { format!(" · {models}") }
    );
    let i = &summary.index;
    println!(
        "  {:<9} {} · {} files indexed{}",
        style("index").bold(),
        latency(&i.latency, "run", "runs"),
        i.files_indexed,
        i.last.as_deref().map(|at| format!(" · last {}", short(at))).unwrap_or_default()
    );
    let e = &summary.errors;
    println!(
        "  {:<9} {}{}",
        style("errors").bold(),
        e.count,
        if e.count > 0 { format!(" · {}", top(&e.commands)) } else { String::new() }
    );
    for failure in &e.recent {
        println!("    {} {}: {}", style(short(&failure.at)).dim(), failure.command, style(&failure.message).dim());
    }
    if e.count > 0 {
        ui::hint("report one with `brok bug`");
    }
}

fn latency(latency: &Latency, one: &str, many: &str) -> String {
    let what = if latency.count == 1 { one } else { many };
    match (latency.p50_ms, latency.p95_ms) {
        (Some(p50), Some(p95)) => format!("{} {what} · p50 {} · p95 {}", latency.count, duration(p50), duration(p95)),
        _ => format!("{} {what}", latency.count),
    }
}

/// `42 ms`, `1.8 s`, `2.5 min`.
fn duration(ms: u64) -> String {
    match ms {
        0..1_000 => format!("{ms} ms"),
        1_000..60_000 => format!("{:.1} s", ms as f64 / 1_000.0),
        _ => format!("{:.1} min", ms as f64 / 60_000.0),
    }
}

/// `qwen (12), ollama (3)`: the three most frequent.
fn top(counts: &BTreeMap<String, usize>) -> String {
    let mut entries: Vec<(&String, &usize)> = counts.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    entries.iter().take(3).map(|(k, v)| format!("{k} ({v})")).collect::<Vec<_>>().join(", ")
}

/// `2026-11-03T10:12:01Z` -> `2026-11-03 10:12`.
fn short(at: &str) -> String {
    at.get(..16).unwrap_or(at).replace('T', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: &str, ms: Option<u64>, data: serde_json::Value) -> Event {
        Event {
            at: "2026-11-03T10:00:00Z".into(),
            kind: kind.into(),
            ms,
            data: data.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn summary() {
        let events = vec![
            event("search", Some(10), json!({ "hits": 8, "semantic": true, "links": 2, "memories": 1 })),
            event("search", Some(30), json!({ "hits": 0, "semantic": false, "links": 0, "memories": 0 })),
            event(
                "answer",
                Some(4_000),
                json!({ "ok": true, "provider": "ollama", "model": "qwen", "first_token_ms": 900 }),
            ),
            event("answer", Some(9_000), json!({ "ok": false, "skill": "code-review" })),
            event("index", Some(200), json!({ "files_indexed": 12 })),
            event("error", None, json!({ "command": "ask", "message": "no answer" })),
        ];
        let summary = summarize(&events, 7);
        assert_eq!(summary.events, 6);
        assert_eq!(summary.search.latency, Latency { count: 2, p50_ms: Some(10), p95_ms: Some(30) });
        assert_eq!((summary.search.empty, summary.search.semantic, summary.search.with_links), (1, 1, 1));
        assert_eq!(summary.answers.latency.count, 1);
        assert_eq!(summary.answers.failed, 1);
        assert_eq!(summary.answers.first_token.p50_ms, Some(900));
        assert_eq!(summary.answers.models.get("ollama/qwen"), Some(&1));
        assert_eq!(summary.answers.skills.get("code-review"), Some(&1));
        assert_eq!(summary.index.files_indexed, 12);
        assert_eq!(summary.errors.recent[0].message, "no answer");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(42), "42 ms");
        assert_eq!(duration(1_800), "1.8 s");
        assert_eq!(duration(150_000), "2.5 min");
    }
}
