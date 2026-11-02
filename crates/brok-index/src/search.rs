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
//!
//! Linked workspaces (`workspace.links`) are searched the same way, scaled
//! by `search.link_weight`, with their paths shown relative to this
//! workspace (`../checkout/src/...`). Memories join the ranking as a list
//! of their own (see `memory.rs`).

use std::collections::HashMap;

use anyhow::Result;
use brok_sdk::payload::{SearchHit, SearchResults};

use crate::embed::EmbedKind;
use crate::index::Index;

/// RRF damping constant (60 in the original paper).
pub(crate) const RRF_K: f32 = 60.0;
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

/// A ranked chunk: (partition index, chunk id, score).
type Candidate = (usize, String, f32);

impl Index {
    /// Runs a hybrid search over every partition, the linked workspaces and
    /// the memories.
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
        let model = self.embedder.as_ref().map(|e| e.model_id().to_owned());
        let vector = query_vector.as_deref().zip(model.as_deref());

        let mut semantic = false;
        let own = self.candidates(&query.text, vector, limit, focus.as_deref(), 1.0, &mut semantic).await?;
        let mut hits = self.hits(own, "", None).await?;

        let mut links = self.links.lock().await;
        for link in links.iter_mut() {
            link.refresh(&self.config).await;
            let Some(index) = &link.index else { continue };
            let weight = self.config.search.link_weight;
            let found = index.candidates(&query.text, vector, limit, None, weight, &mut semantic).await?;
            hits.extend(index.hits(found, &link.prefix, Some(&link.link.name)).await?);
        }
        drop(links);

        if self.config.memory.enabled {
            // A memory found by text is worth a chunk found by one of the
            // lists code is ranked with; a few at most, so code still shows.
            let lists = if query_vector.is_some() { 2 } else { 1 };
            hits.extend(self.memory.search(&query.text, limit.div_ceil(3).max(2), lists));
        }

        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(SearchResults { hits, semantic })
    }

    /// The best `limit` chunks over every partition, each scaled by
    /// `weight` (and by `search.cold_weight` outside the `focus`
    /// partition). Vectors are searched only in partitions built with the
    /// query's embedding model.
    pub(crate) async fn candidates(
        &self,
        text: &str,
        vector: Option<(&[f32], &str)>,
        limit: usize,
        focus: Option<&str>,
        weight: f32,
        semantic: &mut bool,
    ) -> Result<Vec<Candidate>> {
        let mut candidates = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            let lexical: Vec<String> =
                part.bm25.search(text, limit * OVERFETCH).into_iter().map(|(id, _)| id).collect();
            let compatible = |v: &[f32], model: &str| {
                part.store.dims() == Some(v.len()) && part.embedding().is_none_or(|e| e.model == model)
            };
            let vectors: Vec<String> = match vector {
                Some((v, model)) if compatible(v, model) => {
                    *semantic = true;
                    part.store.vector_search(v, limit * OVERFETCH).await?.into_iter().map(|(id, _)| id).collect()
                }
                _ => Vec::new(),
            };
            let lists = usize::from(!lexical.is_empty()) + usize::from(!vectors.is_empty());
            if lists == 0 {
                continue;
            }
            let weight = match focus {
                Some(id) if id != part.partition.id => weight * self.config.search.cold_weight,
                _ => weight,
            };
            // Normalize so that rank 1 in every available list scores 1.0.
            let best = lists as f32 / (RRF_K + 1.0);
            for (id, score) in rrf(&[&lexical, &vectors]) {
                candidates.push((i, id, weight * score / best));
            }
        }
        candidates.sort_by(|a, b| b.2.total_cmp(&a.2));
        candidates.truncate(limit);
        Ok(candidates)
    }

    /// The chunks behind `candidates`, as hits. A linked workspace's hits
    /// get `prefix` on their paths and `@<link>` in their partition.
    pub(crate) async fn hits(
        &self,
        candidates: Vec<Candidate>,
        prefix: &str,
        link: Option<&str>,
    ) -> Result<Vec<SearchHit>> {
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

        Ok(candidates
            .into_iter()
            .filter_map(|(i, id, score)| {
                let chunk = chunks.remove(&(i, id))?;
                let partition = &self.parts[i].partition.id;
                Some(SearchHit {
                    chunk_id: chunk.id,
                    partition: match link {
                        Some(name) if partition == "root" => format!("@{name}"),
                        Some(name) => format!("@{name}/{partition}"),
                        None => partition.clone(),
                    },
                    path: format!("{prefix}{}", chunk.path),
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
            .collect())
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
