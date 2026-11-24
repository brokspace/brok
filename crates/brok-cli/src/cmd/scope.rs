//! Scoped search across a Meta Workspace (ADR 0014), shared by `brok search`
//! and the MCP `brok_search` tool.

use brok_core::Workspace;
use brok_core::members::Member;
use brok_index::{Access, Index, SearchQuery};

/// Where `brok search` looks (ADR 0014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Scope {
    /// This workspace only (the default; unchanged behavior).
    Repository,
    /// This workspace and its Meta Workspace members that have a Brokspace.
    Workspace,
}

/// A hit and the member it comes from (`None`: this workspace).
pub struct Found {
    pub member: Option<String>,
    pub hit: brok_sdk::prelude::SearchHit,
    /// Rank score: the hit's score relative to the best of its own index, so
    /// indexes with different score scales compete fairly.
    pub rank: f32,
}

/// Searches the members of `workspace` that have their own index, read-only.
/// Returns the hits and a note for each member that was skipped.
pub async fn search_members(workspace: &Workspace, text: &str, limit: usize) -> (Vec<Found>, Vec<String>) {
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    for member in Member::all(workspace) {
        let opened = match member.searchable(workspace) {
            Some(opened) => opened,
            None => {
                skipped.push(if member.is_git() {
                    format!("{}: not cloned yet (run `brok workspace sync {}`)", member.id, member.id)
                } else {
                    format!("{}: no Brokspace of its own (run `brok init` there)", member.id)
                });
                continue;
            }
        };
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
pub fn rank(hits: Vec<brok_sdk::prelude::SearchHit>, member: Option<String>) -> Vec<Found> {
    let best = hits.iter().map(|h| h.score).fold(0.0f32, f32::max);
    hits.into_iter()
        .map(|hit| {
            let rank = if best > 0.0 { hit.score / best } else { 0.0 };
            Found { member: member.clone(), hit, rank }
        })
        .collect()
}

/// Best first; on a tie this workspace comes before its members.
pub fn merge(mut all: Vec<Found>, limit: usize) -> Vec<Found> {
    all.sort_by(|a, b| {
        b.rank
            .total_cmp(&a.rank)
            .then_with(|| a.member.is_some().cmp(&b.member.is_some()))
            .then_with(|| b.hit.score.total_cmp(&a.hit.score))
    });
    all.truncate(limit);
    all
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
