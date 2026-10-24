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
