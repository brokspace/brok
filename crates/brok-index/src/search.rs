//! Federated hybrid search.
//!
//! For each partition, BM25 and vector search each produce a ranked list
//! that is fused with Reciprocal Rank Fusion (RRF): a chunk's score is
//! `Σ 1 / (k + rank)` over the lists it appears in. RRF needs no score
//! calibration between BM25 and cosine similarity, and degrades gracefully
//! to pure BM25 when no vectors are available.
//!
//! Partitions are then merged: the *focused* partition (the hot submodule
//! the user works in) keeps full weight and every other partition is scaled
//! by `search.cold_weight`, so local context wins ties while the rest of
//! the workspace still resolves cross-repository references.

use std::collections::HashMap;

use anyhow::Result;
use brok_sdk::payload::{SearchHit, SearchResults};

use crate::embed::EmbedKind;
use crate::index::Index;

/// RRF damping constant (60 in the original paper).
const RRF_K: f32 = 60.0;
/// Candidates fetched per list, relative to the requested limit.
const OVERFETCH: usize = 4;

/// A search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free text.
    pub text: String,
    /// Maximum number of hits.
    pub limit: usize,
    /// Partition id, or a workspace relative path inside the partition to
    /// prioritise.
    pub focus: Option<String>,
}

impl Index {
    /// Runs a hybrid search over every partition.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResults> {
        let limit = query.limit.max(1);
        let focus = query
            .focus
            .as_deref()
            .map(|f| self.partitions.get(f).map_or_else(|| self.partitions.resolve(f).id.clone(), |p| p.id.clone()));

        let query_vector = match &self.embedder {
            Some(embedder) => {
                let embedder = embedder.clone();
                let text = vec![query.text.clone()];
                let mut vectors =
                    tokio::task::spawn_blocking(move || embedder.embed(&text, EmbedKind::Query)).await??;
                vectors.pop()
            }
            None => None,
        };

        let mut semantic = false;
        let mut candidates: Vec<(usize, String, f32)> = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            let lexical: Vec<String> =
                part.bm25.search(&query.text, limit * OVERFETCH).into_iter().map(|(id, _)| id).collect();
            let vector: Vec<String> = match &query_vector {
                Some(v) if part.store.dims() == Some(v.len()) => {
                    semantic = true;
                    part.store.vector_search(v, limit * OVERFETCH).await?.into_iter().map(|(id, _)| id).collect()
                }
                _ => Vec::new(),
            };
            let lists = usize::from(!lexical.is_empty()) + usize::from(!vector.is_empty());
            if lists == 0 {
                continue;
            }
            let weight = match &focus {
                Some(id) if *id != part.partition.id => self.config.search.cold_weight,
                _ => 1.0,
            };
            // Normalize so that rank 1 in every available list scores 1.0.
            let best = lists as f32 / (RRF_K + 1.0);
            for (id, score) in rrf(&[&lexical, &vector]) {
                candidates.push((i, id, weight * score / best));
            }
        }

        candidates.sort_by(|a, b| b.2.total_cmp(&a.2));
        candidates.truncate(limit);

        let mut wanted: HashMap<usize, Vec<String>> = HashMap::new();
        for (i, id, _) in &candidates {
            wanted.entry(*i).or_default().push(id.clone());
        }
        let mut chunks = HashMap::new();
        for (i, ids) in wanted {
            for chunk in self.parts[i].store.get(&ids).await? {
                chunks.insert((i, chunk.id.clone()), chunk);
            }
        }

        let hits = candidates
            .into_iter()
            .filter_map(|(i, id, score)| {
                let chunk = chunks.remove(&(i, id))?;
                Some(SearchHit {
                    chunk_id: chunk.id,
                    partition: self.parts[i].partition.id.clone(),
                    path: chunk.path,
                    language: chunk.language,
                    kind: chunk.kind.as_str().to_owned(),
                    symbol: chunk.symbol,
                    signature: chunk.signature,
                    start_line: chunk.start_line,
                    end_line: chunk.end_line,
                    score,
                    content: chunk.content,
                })
            })
            .collect();
        Ok(SearchResults { hits, semantic })
    }
}

/// Reciprocal Rank Fusion of ranked id lists.
pub(crate) fn rrf(lists: &[&[String]]) -> Vec<(String, f32)> {
    let mut scores: HashMap<&str, f32> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(id.as_str()).or_default() += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut fused: Vec<(String, f32)> = scores.into_iter().map(|(id, s)| (id.to_owned(), s)).collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn rrf_rewards_agreement() {
        let lexical = ids(&["a", "b", "c"]);
        let vector = ids(&["c", "a", "d"]);
        let fused = rrf(&[&lexical, &vector]);
        assert_eq!(fused[0].0, "a");
        assert_eq!(fused[1].0, "c");
        assert_eq!(fused.len(), 4);
    }

    #[test]
    fn rrf_single_list_keeps_order() {
        let fused = rrf(&[&ids(&["x", "y"]), &[]]);
        assert_eq!(fused.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), vec!["x", "y"]);
    }
}
