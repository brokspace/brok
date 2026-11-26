//! End-to-end indexing and search on a temporary workspace, for every
//! store backend compiled into this build.

use std::sync::Arc;

use anyhow::Result;
use brok_core::config::StoreBackend;
use brok_core::init::{InitOptions, init_workspace};
use brok_core::{BrokConfig, Workspace};
use brok_index::{Access, EmbedKind, Embedder, Index, SearchQuery};

/// Deterministic bag-of-words embedder: good enough to exercise the vector
/// path without downloading a model.
struct HashEmbedder;

impl Embedder for HashEmbedder {
    fn dims(&self) -> usize {
        16
    }
    fn model_id(&self) -> &str {
        "test-hash"
    }
    fn embed(&self, texts: &[String], _kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut v = vec![0f32; 16];
                for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
                    let h =
                        word.to_lowercase().bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
                    v[(h % 16) as usize] += 1.0;
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                v.iter_mut().for_each(|x| *x /= norm);
                v
            })
            .collect())
    }
}

fn workspace(store: StoreBackend, semantic: bool) -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = init_workspace(dir.path(), InitOptions::default()).unwrap().workspace;
    let mut config: BrokConfig = ws.config().clone();
    config.index.store = store;
    config.embeddings.enabled = semantic;
    ws.save_config(config).unwrap();

    let root = ws.root().to_path_buf();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/config.rs"),
        "/// Loads the configuration file.\npub fn load_config(path: &str) -> String {\n    std::fs::read_to_string(path).unwrap()\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/render.py"), "def render_page(page):\n    return f'<h1>{page}</h1>'\n").unwrap();
    std::fs::write(root.join("README.md"), "# Demo\n\nHow to configure the server.\n").unwrap();
    std::fs::write(root.join("logo.bin"), [0u8, 159, 146, 150]).unwrap();
    std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    std::fs::create_dir_all(root.join("ignored")).unwrap();
    std::fs::write(root.join("ignored/secret.rs"), "fn load_config_secret() {}").unwrap();
    // Folders brok generated for coding agents are not workspace knowledge.
    std::fs::create_dir_all(root.join(".claude/skills/tdd")).unwrap();
    std::fs::write(root.join(".claude/skills/tdd/.brokspace-managed"), "").unwrap();
    std::fs::write(root.join(".claude/skills/tdd/SKILL.md"), "load config skill").unwrap();
    (dir, Workspace::open(&root).unwrap())
}

fn embedder(semantic: bool) -> Option<Arc<dyn Embedder>> {
    semantic.then(|| Arc::new(HashEmbedder) as Arc<dyn Embedder>)
}

async fn run(store: StoreBackend, semantic: bool) {
    let (_dir, ws) = workspace(store, semantic);
    let mut index = Index::open(&ws, embedder(semantic), Access::ReadWrite).await.unwrap();

    let report = index.index_all(false, &|_| {}).await.unwrap();
    assert_eq!(report.files_indexed, 4, "{report:?}"); // 3 sources + .gitignore; binary and ignored skipped
    assert!(report.chunks >= 4);

    let query = SearchQuery { text: "load config".into(), limit: 3, focus: None };
    let results = index.search(&query).await.unwrap();
    assert_eq!(results.semantic, semantic);
    let top = &results.hits[0];
    assert_eq!(top.path, "src/config.rs");
    assert_eq!(top.symbol.as_deref(), if cfg!(feature = "ast") { Some("load_config") } else { None });
    assert!(top.content.contains("read_to_string"));
    assert!(results.hits.iter().all(|h| !h.path.starts_with("ignored/") && !h.path.starts_with(".claude/")));

    // Nothing changed: nothing re-indexed.
    let again = index.index_all(false, &|_| {}).await.unwrap();
    assert_eq!(again.files_indexed, 0, "{again:?}");

    // Incremental update and deletion through the change API.
    let root = ws.root();
    std::fs::write(root.join("src/render.py"), "def render_table(rows):\n    return rows\n").unwrap();
    std::fs::remove_file(root.join("README.md")).unwrap();
    let report = index
        .apply_changes(&["src/render.py".into(), ".claude/skills/tdd/SKILL.md".into()], &["README.md".into()])
        .await
        .unwrap();
    assert_eq!((report.files_indexed, report.files_removed), (1, 1), "managed folders stay out");

    let results = index.search(&SearchQuery { text: "render table".into(), limit: 5, focus: None }).await.unwrap();
    assert_eq!(results.hits[0].path, "src/render.py");
    assert!(results.hits.iter().all(|h| h.path != "README.md"));

    let status = index.status().await.unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].files, 3);
    assert_eq!(status[0].semantic, semantic);
    drop(index);

    // A second writer is refused while the first one is alive.
    let writer = Index::open(&ws, embedder(semantic), Access::ReadWrite).await.unwrap();
    assert!(Index::open(&ws, embedder(semantic), Access::ReadWrite).await.is_err());
    // ...but readers are fine, and see the committed data.
    let reader = Index::open(&ws, None, Access::ReadOnly).await.unwrap();
    let results = reader.search(&SearchQuery { text: "render table".into(), limit: 1, focus: None }).await.unwrap();
    assert_eq!(results.hits[0].path, "src/render.py");
    drop(writer);

    // Rebuild from scratch.
    let mut index = Index::open(&ws, embedder(semantic), Access::ReadWrite).await.unwrap();
    let rebuilt = index.index_all(true, &|_| {}).await.unwrap();
    assert_eq!(rebuilt.files_indexed, 3);
}

#[tokio::test]
async fn arrow_store_bm25_only() {
    run(StoreBackend::Arrow, false).await;
}

#[tokio::test]
async fn arrow_store_hybrid() {
    run(StoreBackend::Arrow, true).await;
}

#[cfg(feature = "lancedb")]
#[tokio::test]
async fn lancedb_store_bm25_only() {
    run(StoreBackend::Lancedb, false).await;
}

#[cfg(feature = "lancedb")]
#[tokio::test]
async fn lancedb_store_hybrid() {
    run(StoreBackend::Lancedb, true).await;
}

#[tokio::test]
async fn writer_requires_the_model_when_embeddings_are_enabled() {
    let (_dir, ws) = workspace(StoreBackend::Arrow, true);
    let err = Index::open(&ws, None, Access::ReadWrite).await.err().expect("must fail");
    assert!(err.to_string().contains("embedding model"), "{err}");
}

/// Embeds like [`HashEmbedder`] but fails after `budget` calls, as a run that
/// is stopped halfway.
struct StopsAfter {
    budget: std::sync::atomic::AtomicUsize,
}

impl Embedder for StopsAfter {
    fn dims(&self) -> usize {
        HashEmbedder.dims()
    }
    fn model_id(&self) -> &str {
        HashEmbedder.model_id()
    }
    fn embed(&self, texts: &[String], kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        use std::sync::atomic::Ordering;
        if kind == EmbedKind::Document
            && self.budget.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_err()
        {
            anyhow::bail!("stopped");
        }
        HashEmbedder.embed(texts, kind)
    }
}

fn many_files(root: &std::path::Path, count: usize) {
    for i in 0..count {
        std::fs::write(root.join(format!("src/item{i:03}.rs")), format!("pub fn item_{i}_total() -> u32 {{ {i} }}\n"))
            .unwrap();
    }
}

async fn counts(index: &Index) -> (usize, usize, usize, bool) {
    let status = index.status().await.unwrap();
    (status[0].files, status[0].chunks, status[0].waiting, status[0].semantic)
}

fn query(text: &str) -> SearchQuery {
    SearchQuery { text: text.into(), limit: 5, focus: None }
}

async fn interrupted_indexing_resumes(store: StoreBackend) {
    let (_dir, ws) = workspace(store, true);
    many_files(ws.root(), 300);
    // Enough for two saves of about 64 chunks, then it stops.
    let stops = Arc::new(StopsAfter { budget: 2.into() });
    let mut index = Index::open(&ws, Some(stops), Access::ReadWrite).await.unwrap();
    index.set_checkpoint(std::time::Duration::ZERO);

    // The text pass alone: everything is found by keywords, nothing by meaning.
    index.index_text(false, &|_| {}).await.unwrap();
    let (files, chunks, waiting, semantic) = counts(&index).await;
    assert_eq!((waiting, semantic), (chunks, false));
    let found = index.search(&query("item 217 total")).await.unwrap();
    assert!(!found.semantic);
    assert!(found.hits.iter().any(|h| h.path == "src/item217.rs"), "{found:?}");

    index.fill_vectors(&|_| {}).await.expect_err("stopped halfway");
    let (_, _, left, _) = counts(&index).await;
    assert!(left > 0 && left < chunks, "saved work is kept: {left} of {chunks}");
    drop(index); // as if the process ended

    // A new run, even with no file changed, embeds only what is left.
    let mut index = Index::open(&ws, embedder(true), Access::ReadWrite).await.unwrap();
    assert_eq!(index.waiting_chunks(), left);
    let embedded = std::sync::atomic::AtomicUsize::new(0);
    let report = index
        .index_all(false, &|event| {
            if let brok_index::Progress::TextReady { waiting } = event {
                embedded.store(waiting, std::sync::atomic::Ordering::SeqCst);
            }
        })
        .await
        .unwrap();
    assert_eq!((report.files_indexed, embedded.into_inner()), (0, left));
    assert_eq!(counts(&index).await, (files, chunks, 0, true), "nothing lost or duplicated");
    let reader = Index::open(&ws, embedder(true), Access::ReadOnly).await.unwrap();
    let found = reader.search(&query("item 217 total")).await.unwrap();
    assert!(found.semantic);
    let paths: Vec<_> = found.hits.iter().map(|h| h.path.as_str()).collect();
    assert_eq!(paths.iter().filter(|p| **p == "src/item217.rs").count(), 1, "{paths:?}");
}

async fn files_changed_between_the_passes_end_up_right(store: StoreBackend) {
    let (_dir, ws) = workspace(store, true);
    many_files(ws.root(), 3);
    let mut index = Index::open(&ws, embedder(true), Access::ReadWrite).await.unwrap();
    index.index_text(false, &|_| {}).await.unwrap();
    let job = index.vector_job(10_000).unwrap();
    // While the job embeds: one file is edited and indexed, one deleted
    // unnoticed, one edited unnoticed.
    let root = ws.root();
    std::fs::write(root.join("src/item000.rs"), "pub fn renamed_alpha() {}\n").unwrap();
    index.apply_changes_text(&["src/item000.rs".into()], &[]).await.unwrap();
    std::fs::remove_file(root.join("src/item001.rs")).unwrap();
    std::fs::write(root.join("src/item002.rs"), "pub fn renamed_gamma() {}\n").unwrap();
    let batch = job.run().await.unwrap();
    index.save_vectors(batch).await.unwrap();
    index.fill_vectors(&|_| {}).await.unwrap();
    index.index_all(false, &|_| {}).await.unwrap();

    assert_eq!(index.waiting_chunks(), 0);
    for (text, path) in [("renamed alpha", "src/item000.rs"), ("renamed gamma", "src/item002.rs")] {
        let found = index.search(&query(text)).await.unwrap();
        assert_eq!(found.hits[0].path, path, "{text}: {found:?}");
        assert!(found.hits[0].content.contains("renamed"), "{found:?}");
    }
    let found = index.search(&query("item 1 total")).await.unwrap();
    assert!(found.hits.iter().all(|h| h.path != "src/item001.rs" && !h.content.contains("item_0_total")));
}

#[tokio::test]
async fn arrow_interrupted_indexing_resumes() {
    interrupted_indexing_resumes(StoreBackend::Arrow).await;
}

#[tokio::test]
async fn arrow_files_changed_between_the_passes_end_up_right() {
    files_changed_between_the_passes_end_up_right(StoreBackend::Arrow).await;
}

#[cfg(feature = "lancedb")]
#[tokio::test]
async fn lancedb_interrupted_indexing_resumes() {
    interrupted_indexing_resumes(StoreBackend::Lancedb).await;
}

#[cfg(feature = "lancedb")]
#[tokio::test]
async fn lancedb_files_changed_between_the_passes_end_up_right() {
    files_changed_between_the_passes_end_up_right(StoreBackend::Lancedb).await;
}
