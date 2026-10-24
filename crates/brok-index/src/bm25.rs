//! A compact, incremental BM25 full-text index.
//!
//! This is the text half of hybrid search, and the whole of it when
//! embeddings are disabled. It is tuned for code:
//!
//! - identifiers are indexed whole *and* split on `snake_case` /
//!   `camelCase` boundaries, so `parseConfig`, `parse_config` and
//!   "parse config" all find each other;
//! - removals are lazy (tombstones) and the index compacts itself once a
//!   quarter of the documents are dead.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const K1: f32 = 1.2;
const B: f32 = 0.75;
const FORMAT_VERSION: u32 = 1;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of", "on", "or", "that", "the",
    "this", "to", "was", "with",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Doc {
    chunk_id: String,
    path: String,
    len: u32,
    live: bool,
}

/// BM25 index over chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Index {
    version: u32,
    docs: Vec<Doc>,
    /// term → (doc slot, term frequency)
    postings: HashMap<String, Vec<(u32, u16)>>,
    by_path: HashMap<String, Vec<u32>>,
    live: u32,
    total_len: u64,
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            docs: Vec::new(),
            postings: HashMap::new(),
            by_path: HashMap::new(),
            live: 0,
            total_len: 0,
        }
    }
}

impl Bm25Index {
    /// Loads an index file, or returns an empty index when it is missing,
    /// unreadable or from another format version.
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else { return Self::default() };
        match postcard::from_bytes::<Self>(&bytes) {
            Ok(index) if index.version == FORMAT_VERSION => index,
            _ => {
                tracing::warn!(path = %path.display(), "BM25 index is unreadable; it will be rebuilt");
                Self::default()
            }
        }
    }

    /// Persists the index atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = postcard::to_stdvec(self).context("serializing BM25 index")?;
        brok_core::fsutil::write_atomic(path, &bytes)?;
        Ok(())
    }

    /// Number of live documents.
    pub fn len(&self) -> usize {
        self.live as usize
    }

    /// Whether the index has no live document.
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Indexes one chunk.
    pub fn add(&mut self, chunk_id: &str, path: &str, text: &str) {
        let slot = u32::try_from(self.docs.len()).expect("more than u32::MAX documents");
        let mut frequencies: HashMap<String, u16> = HashMap::new();
        let mut len = 0u32;
        for token in tokenize(text) {
            len += 1;
            let tf = frequencies.entry(token).or_default();
            *tf = tf.saturating_add(1);
        }
        for (term, tf) in frequencies {
            self.postings.entry(term).or_default().push((slot, tf));
        }
        self.docs.push(Doc { chunk_id: chunk_id.to_owned(), path: path.to_owned(), len, live: true });
        self.by_path.entry(path.to_owned()).or_default().push(slot);
        self.live += 1;
        self.total_len += u64::from(len);
    }

    /// Removes every chunk of `path`.
    pub fn remove_path(&mut self, path: &str) {
        let Some(slots) = self.by_path.remove(path) else { return };
        for slot in slots {
            let doc = &mut self.docs[slot as usize];
            if doc.live {
                doc.live = false;
                self.live -= 1;
                self.total_len -= u64::from(doc.len);
            }
        }
        let dead = self.docs.len() - self.live as usize;
        if dead > 1024 && dead * 4 > self.docs.len() {
            self.compact();
        }
    }

    /// Top `k` chunk ids for `query`, best first.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        if self.live == 0 || k == 0 {
            return Vec::new();
        }
        let n = self.live as f32;
        let avg_len = (self.total_len as f32 / self.live as f32).max(1.0);
        let mut terms: Vec<String> = tokenize(query).collect();
        terms.sort();
        terms.dedup();

        let mut scores: HashMap<u32, f32> = HashMap::new();
        for term in &terms {
            let Some(postings) = self.postings.get(term) else { continue };
            let live_postings: Vec<&(u32, u16)> =
                postings.iter().filter(|(slot, _)| self.docs[*slot as usize].live).collect();
            if live_postings.is_empty() {
                continue;
            }
            let df = live_postings.len() as f32;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for &&(slot, tf) in &live_postings {
                let tf = f32::from(tf);
                let len = self.docs[slot as usize].len as f32;
                let score = idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / avg_len));
                *scores.entry(slot).or_default() += score;
            }
        }

        let mut ranked: Vec<(u32, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(k);
        ranked.into_iter().map(|(slot, score)| (self.docs[slot as usize].chunk_id.clone(), score)).collect()
    }

    /// Drops tombstones and renumbers documents.
    pub fn compact(&mut self) {
        let mut remap = vec![u32::MAX; self.docs.len()];
        let mut docs = Vec::with_capacity(self.live as usize);
        for (old, doc) in std::mem::take(&mut self.docs).into_iter().enumerate() {
            if doc.live {
                remap[old] = docs.len() as u32;
                docs.push(doc);
            }
        }
        self.docs = docs;
        self.postings.retain(|_, postings| {
            postings.retain_mut(|(slot, _)| {
                let new = remap[*slot as usize];
                *slot = new;
                new != u32::MAX
            });
            !postings.is_empty()
        });
        for slots in self.by_path.values_mut() {
            slots.retain_mut(|slot| {
                let new = remap[*slot as usize];
                *slot = new;
                new != u32::MAX
            });
        }
        self.by_path.retain(|_, slots| !slots.is_empty());
    }
}

/// Code-aware tokenizer: lowercase alphanumeric words, plus the parts of
/// `snake_case` and `camelCase` identifiers.
pub fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .flat_map(|word| {
            let parts = split_identifier(word);
            let whole = (parts.len() > 1).then(|| word.trim_matches('_').to_lowercase());
            whole.into_iter().chain(parts)
        })
        .filter(|token| token.len() > 1 && !STOPWORDS.contains(&token.as_str()))
}

fn split_identifier(word: &str) -> Vec<String> {
    let mut parts = Vec::new();
    for segment in word.split('_').filter(|s| !s.is_empty()) {
        let chars: Vec<char> = segment.chars().collect();
        let mut start = 0;
        for i in 1..chars.len() {
            let (prev, cur) = (chars[i - 1], chars[i]);
            let next_is_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            let boundary = (prev.is_lowercase() && cur.is_uppercase())
                || (prev.is_uppercase() && cur.is_uppercase() && next_is_lower);
            if boundary {
                parts.push(chars[start..i].iter().collect::<String>().to_lowercase());
                start = i;
            }
        }
        parts.push(chars[start..].iter().collect::<String>().to_lowercase());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_splits_identifiers() {
        let tokens: Vec<String> = tokenize("fn parseHTTPConfig(user_id: u32) -> the Result").collect();
        assert_eq!(
            tokens,
            vec!["fn", "parsehttpconfig", "parse", "http", "config", "user_id", "user", "id", "u32", "result"]
        );
    }

    #[test]
    fn ranks_relevant_documents_first() {
        let mut index = Bm25Index::default();
        index.add("1", "a.rs", "fn parse_config(path: &Path) -> Config");
        index.add("2", "b.rs", "fn render_page(page: Page) -> Html");
        index.add("3", "c.rs", "struct Config { verbose: bool }");
        let hits = index.search("parse config", 10);
        assert_eq!(hits[0].0, "1");
        assert_eq!(hits.len(), 2);
        assert!(index.search("nothing matches", 10).is_empty());
    }

    #[test]
    fn removal_and_compaction() {
        let mut index = Bm25Index::default();
        for i in 0..3000 {
            index.add(&i.to_string(), &format!("f{i}.rs"), &format!("token{} shared", i % 3));
        }
        for i in 0..2000 {
            index.remove_path(&format!("f{i}.rs"));
        }
        assert_eq!(index.len(), 1000);
        assert!(index.docs.len() < 3000, "compaction should have run");
        let hits = index.search("shared", 5000);
        assert_eq!(hits.len(), 1000);
        assert!(hits.iter().all(|(id, _)| id.parse::<u32>().unwrap() >= 2000));
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bm25.bin");
        let mut index = Bm25Index::default();
        index.add("1", "a.rs", "hello world");
        index.save(&path).unwrap();
        let loaded = Bm25Index::load(&path);
        assert_eq!(loaded.search("hello", 1)[0].0, "1");
        assert!(Bm25Index::load(&dir.path().join("missing")).is_empty());
    }
}
