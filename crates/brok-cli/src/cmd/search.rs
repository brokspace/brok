//! `brok search`

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use brok_core::Workspace;
use brok_core::members::Member;
use brok_index::{Access, Index, SearchQuery};
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

/// Where `brok search` looks (ADR 0014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Scope {
    /// This workspace only (the default; unchanged behavior).
    Repository,
    /// This workspace and its Meta Workspace members that have a Brokspace.
    Workspace,
}

/// A hit and the member it comes from (`None`: this workspace).
struct Found {
    member: Option<String>,
    hit: brok_sdk::prelude::SearchHit,
    /// Rank score: the hit's score relative to the best of its own index, so
    /// indexes with different score scales compete fairly.
    rank: f32,
}

/// Searches the members of `workspace` that have their own index, read-only.
/// Returns the hits and a note for each member that was skipped.
async fn search_members(workspace: &Workspace, text: &str, limit: usize) -> (Vec<Found>, Vec<String>) {
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    for member in Member::all(workspace) {
        if !member.is_workspace() {
            skipped.push(format!("{}: no Brokspace of its own (run `brok init` there)", member.id));
            continue;
        }
        let opened = Workspace::open(&member.root);
        let results = match opened {
            Ok(ws) => match Index::open(&ws, super::embedder_for_search(&ws), Access::ReadOnly).await {
                Ok(index) => index.search(&SearchQuery { text: text.to_owned(), limit, focus: None }).await,
                Err(e) => Err(e),
            },
            Err(e) => {
                skipped.push(format!("{}: {e:#}", member.id));
                continue;
            }
        };
        match results {
            Ok(r) => found.extend(rank(r.hits, Some(member.id))),
            Err(e) => skipped.push(format!("{}: {e:#}", member.id)),
        }
    }
    (found, skipped)
}

/// Scores `hits` relative to the best one.
fn rank(hits: Vec<brok_sdk::prelude::SearchHit>, member: Option<String>) -> Vec<Found> {
    let best = hits.iter().map(|h| h.score).fold(0.0f32, f32::max);
    hits.into_iter()
        .map(|hit| {
            let rank = if best > 0.0 { hit.score / best } else { 0.0 };
            Found { member: member.clone(), hit, rank }
        })
        .collect()
}

/// Best first; on a tie this workspace comes before its members.
fn merge(mut all: Vec<Found>, limit: usize) -> Vec<Found> {
    all.sort_by(|a, b| {
        b.rank
            .total_cmp(&a.rank)
            .then_with(|| a.member.is_some().cmp(&b.member.is_some()))
            .then_with(|| b.hit.score.total_cmp(&a.hit.score))
    });
    all.truncate(limit);
    all
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(path: &str, score: f32) -> brok_sdk::prelude::SearchHit {
        brok_sdk::prelude::SearchHit {
            chunk_id: path.into(),
            partition: "root".into(),
            path: path.into(),
            language: "rust".into(),
            kind: "function".into(),
            symbol: None,
            signature: None,
            start_line: 1,
            end_line: 2,
            score,
            content: String::new(),
        }
    }

    #[test]
    fn indexes_with_different_score_scales_compete_fairly() {
        let mut all = rank(vec![hit("own-1", 12.0), hit("own-2", 3.0)], None);
        all.extend(rank(vec![hit("api-1", 0.9), hit("api-2", 0.8)], Some("api".into())));
        let order: Vec<_> = merge(all, 3).into_iter().map(|f| f.hit.path).collect();
        // Each index's best hit first (this workspace wins the tie), then the next best.
        assert_eq!(order, ["own-1", "api-1", "api-2"]);
    }

    #[test]
    fn empty_results_rank_without_dividing_by_zero() {
        assert!(rank(Vec::new(), None).is_empty());
        assert_eq!(rank(vec![hit("a", 0.0)], None)[0].rank, 0.0);
    }
}
