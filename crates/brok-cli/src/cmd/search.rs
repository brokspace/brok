//! `brok search`

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use brok_index::{Access, Index, SearchQuery};

use super::scope::{Scope, merge, rank, search_members};
use brok_sdk::payload::{SearchRequest, SearchResults};
use console::style;
use serde_json::{Value, json};

use crate::{daemon, ui};

/// Arguments of `brok search`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// What to look for (natural language or identifiers).
    #[arg(required = true, num_args = 1..)]
    query: Vec<String>,
    /// Number of results.
    #[arg(short = 'n', long)]
    limit: Option<usize>,
    /// Print results as JSON.
    #[arg(long)]
    json: bool,
    /// Print whole chunks instead of a preview.
    #[arg(long)]
    full: bool,
    /// Where to search: this `repository` (default) or the whole `workspace`,
    /// which adds the members registered with `brok workspace add`.
    #[arg(long, value_enum, default_value_t = Scope::Repository)]
    scope: Scope,
}

const PREVIEW_LINES: usize = 6;

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let query = args.query.join(" ");
    let limit = args.limit.unwrap_or(workspace.config().search.limit);
    let focus = super::focus_of(&workspace, cwd);

    // Reuse the running daemon (model already loaded) when there is one.
    let mut results = if daemon::is_running(&workspace).await {
        let (client, _inbox) = daemon::connect(&workspace).await?;
        let request = SearchRequest { query: query.clone(), limit, focus };
        client.call::<SearchRequest, SearchResults>(&request, Duration::from_secs(60)).await?
    } else {
        let index = Index::open(&workspace, super::embedder_for_search(&workspace), Access::ReadOnly).await?;
        index.search(&SearchQuery { text: query.clone(), limit, focus }).await?
    };

    let mut notes = Vec::new();
    let mut owners: Vec<Option<String>> = vec![None; results.hits.len()];
    if args.scope == Scope::Workspace {
        let mut all = rank(std::mem::take(&mut results.hits), None);
        let (members, skipped) = search_members(&workspace, &query, limit).await;
        all.extend(members);
        notes = skipped;
        let merged = merge(all, limit);
        owners = merged.iter().map(|f| f.member.clone()).collect();
        results.hits = merged.into_iter().map(|f| f.hit).collect();
    }

    if args.json {
        let mut value = serde_json::to_value(&results)?;
        if args.scope == Scope::Workspace {
            if let Some(hits) = value.get_mut("hits").and_then(Value::as_array_mut) {
                for (hit, owner) in hits.iter_mut().zip(&owners) {
                    hit["member"] = owner.as_deref().map_or(Value::Null, |id| json!(id));
                }
            }
            value["skipped_members"] = json!(notes);
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    for note in &notes {
        ui::warn(&format!("member skipped, {note}"));
    }
    if results.hits.is_empty() {
        ui::warn("no results (did you run `brok index`?)");
        return Ok(());
    }
    for (hit, owner) in results.hits.iter().zip(&owners) {
        let symbol = hit.symbol.as_deref().map(|s| format!(" {s}")).unwrap_or_default();
        println!(
            "{} {}:{}-{} {}{}",
            style(format!("{:.2}", hit.score)).dim(),
            style(match owner {
                Some(id) => format!("{id}:{}", hit.path),
                None => hit.path.clone(),
            })
            .cyan()
            .bold(),
            hit.start_line,
            hit.end_line,
            style(&hit.kind).magenta(),
            style(symbol).bold()
        );
        let lines: Vec<&str> = hit.content.lines().collect();
        let shown = if args.full { lines.len() } else { lines.len().min(PREVIEW_LINES) };
        for line in &lines[..shown] {
            println!("    {line}");
        }
        if shown < lines.len() {
            println!("    {}", style(format!("… {} more lines", lines.len() - shown)).dim());
        }
        println!();
    }
    if !results.semantic {
        ui::hint("text (BM25) search only; enable embeddings for semantic results");
    }
    Ok(())
}
