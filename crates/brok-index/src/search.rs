//! Federated hybrid search.
//!
//! Every partition of the workspace, every partition of its linked
//! workspaces (`workspace.links`) and the memories contribute candidates to
//! two global ranked lists: text matches (BM25, as a relevance that
//! compares across indexes, see `Bm25Index::search_relevance`) and vector
//! (cosine) matches, each score multiplied by its source's weight:
//!
//! - the *focused* partition (the hot submodule the user works in) keeps
//!   full weight, every other partition is scaled by `search.cold_weight`;
//! - linked workspaces are scaled by `search.link_weight`, and their paths
//!   are shown relative to this workspace (`../checkout/src/...`);
//! - memories only have text matches.
//!
//! The two lists are fused with Reciprocal Rank Fusion (RRF): a chunk's
//! score is `Σ 1 / (k + rank)` over the lists it appears in. RRF needs no
//! calibration between BM25 and cosine similarity, and degrades gracefully
//! to pure BM25 when no vectors are available. Ranking globally (rather
//! than per index) lets a strong match in a linked workspace beat a weak
//! one here.

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

/// Where a candidate comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    /// A partition of this workspace.
    Own(usize),
    /// A partition of a linked workspace: (link, partition).
    Link(usize, usize),
    /// A memory (index in the memory hits).
    Memory(usize),
}

/// The two ranked lists, before fusion: (source, chunk id, weighted score).
#[derive(Default)]
struct Lists {
    lexical: Vec<(Source, String, f32)>,
    vectors: Vec<(Source, String, f32)>,
}

/// A chunk to fetch: (partition index, chunk id, score).
pub(crate) type Candidate = (usize, String, f32);

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

        let mut lists = Lists::default();
        let mut semantic = false;
        self.rank(&query.text, vector, limit, focus.as_deref(), 1.0, &mut semantic, &mut lists, Source::Own).await?;

        let mut links = self.links.lock().await;
        for (l, link) in links.iter_mut().enumerate() {
            link.refresh(&self.config).await;
            if let Some(index) = &link.index {
                let weight = self.config.search.link_weight;
                index
                    .rank(&query.text, vector, limit, None, weight, &mut semantic, &mut lists, |p| Source::Link(l, p))
                    .await?;
            }
        }

        // A few memories at most, so code still shows.
        let memories = if self.config.memory.enabled {
            self.memory.search(&query.text, limit.div_ceil(3).max(2))
        } else {
            Vec::new()
        };
        for (m, hit) in memories.iter().enumerate() {
            lists.lexical.push((Source::Memory(m), hit.chunk_id.clone(), hit.score));
        }

        // Fuse the global lists.
        lists.lexical.sort_by(|a, b| b.2.total_cmp(&a.2));
        lists.vectors.sort_by(|a, b| b.2.total_cmp(&a.2));
        let key = |source: &Source, id: &str| format!("{source:?}\u{0}{id}");
        let mut by_key: HashMap<String, (Source, String)> = HashMap::new();
        let mut ranked = |list: &[(Source, String, f32)]| -> Vec<String> {
            list.iter()
                .map(|(source, id, _)| {
                    let k = key(source, id);
                    by_key.entry(k.clone()).or_insert((*source, id.clone()));
                    k
                })
                .collect()
        };
        let (lexical, vectors) = (ranked(&lists.lexical), ranked(&lists.vectors));
        let available = usize::from(!lexical.is_empty()) + usize::from(!vectors.is_empty());
        // Normalize so that rank 1 in every available list scores 1.0.
        let best = available.max(1) as f32 / (RRF_K + 1.0);
        let fused = rrf(&[&lexical, &vectors]);

        // The best `limit`, grouped by where they come from.
        let mut own: Vec<Candidate> = Vec::new();
        let mut linked: HashMap<usize, Vec<Candidate>> = HashMap::new();
        let mut hits: Vec<SearchHit> = Vec::new();
        for (k, score) in fused.into_iter().take(limit) {
            let Some((source, id)) = by_key.remove(&k) else { continue };
            let score = score / best;
            match source {
                Source::Own(p) => own.push((p, id, score)),
                Source::Link(l, p) => linked.entry(l).or_default().push((p, id, score)),
                Source::Memory(m) => {
                    let mut hit = memories[m].clone();
                    hit.score = score;
                    hits.push(hit);
                }
            }
        }
        hits.extend(self.hits(own, "", None).await?);
        for (l, candidates) in linked {
            if let Some(index) = &links[l].index {
                hits.extend(index.hits(candidates, &links[l].prefix, Some(&links[l].link.name)).await?);
            }
        }
        drop(links);

        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(SearchResults { hits, semantic })
    }

    /// Adds this index's best text and vector matches to `lists`, each
    /// score scaled by `weight` (and by `search.cold_weight` outside the
    /// `focus` partition). Vectors are searched only in partitions built with
    /// the query's embedding model.
    #[allow(clippy::too_many_arguments)]
    async fn rank(
        &self,
        text: &str,
        vector: Option<(&[f32], &str)>,
        limit: usize,
        focus: Option<&str>,
        weight: f32,
        semantic: &mut bool,
        lists: &mut Lists,
        source: impl Fn(usize) -> Source,
    ) -> Result<()> {
        for (i, part) in self.parts.iter().enumerate() {
            let weight = match focus {
                Some(id) if id != part.partition.id => weight * self.config.search.cold_weight,
                _ => weight,
            };
            for (id, relevance) in part.bm25.search_relevance(text, limit * OVERFETCH) {
                lists.lexical.push((source(i), id, relevance * weight));
            }
            let compatible = |v: &[f32], model: &str| {
                part.store.dims() == Some(v.len()) && part.embedding().is_none_or(|e| e.model == model)
            };
            if let Some((v, _)) = vector.filter(|(v, model)| compatible(v, model)) {
                *semantic = true;
                for (id, similarity) in part.store.vector_search(v, limit * OVERFETCH).await? {
                    lists.vectors.push((source(i), id, similarity * weight));
                }
            }
        }
        Ok(())
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
