//! Memories and linked workspaces in search: a `user` service that links
//! the `checkout` service it calls.

use brok_core::config::{LinkConfig, StoreBackend};
use brok_core::init::{InitOptions, init_workspace};
use brok_core::memory::{Kind, NewMemory, Scope};
use brok_core::{Link, MemoryDirs, Workspace};
use brok_index::{Access, Index, SearchQuery, search_memories};

fn user_workspace(dir: &std::path::Path) -> Workspace {
    std::fs::create_dir_all(dir.join("user")).unwrap();
    let mut ws = init_workspace(&dir.join("user"), InitOptions::default()).unwrap().workspace;
    let mut config = ws.config().clone();
    config.index.store = StoreBackend::Arrow;
    config.embeddings.enabled = false;
    config.memory.global = false; // never read the real user's memories
    let mut checkout = LinkConfig::local("../checkout");
    checkout.description = Some("Carts and payments. The user service calls its createOrder API.".into());
    config.workspace.links.insert("checkout".into(), checkout);
    ws.save_config(config).unwrap();
    std::fs::create_dir_all(ws.root().join("src")).unwrap();
    std::fs::write(
        ws.root().join("src/profile.ts"),
        "export function loadProfile(id: string) { return db.users.find(id); }\n",
    )
    .unwrap();
    Workspace::open(ws.root()).unwrap()
}

fn query(text: &str) -> SearchQuery {
    SearchQuery { text: text.into(), limit: 8, focus: None }
}

#[tokio::test]
async fn searches_linked_workspaces_and_memories() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout = tmp.path().join("checkout");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::write(
        checkout.join("src/orders.ts"),
        "export async function createOrder(cart: Cart) {\n  return payments.charge(cart.total);\n}\n",
    )
    .unwrap();
    // A shared memory of the linked workspace (committed there).
    std::fs::create_dir_all(checkout.join(".brokspace-memory")).unwrap();
    std::fs::write(
        checkout.join(".brokspace-memory/retries.md"),
        "# Order retries\n\ncreateOrder is idempotent per cart id.\n",
    )
    .unwrap();

    let user = user_workspace(tmp.path());

    // Not indexed yet: searching still works, and says why the link is missing.
    let index = Index::open(&user, None, Access::ReadOnly).await.unwrap();
    let status = index.link_status().await;
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].root, "../checkout");
    assert!(status[0].error.as_deref().unwrap_or_default().contains("brok link sync checkout"), "{status:?}");

    // `brok link sync`: index the link into user/.brokspace/links/checkout/state.
    let link = Link::all(&user).remove(0);
    let linked = link.workspace(user.config()).unwrap();
    let mut writer = Index::open(&linked, None, Access::ReadWrite).await.unwrap();
    writer.index_all(false, &|_| {}).await.unwrap();
    drop(writer);
    assert!(user.paths().links.join("checkout").join("state").join("databases").is_dir());
    // The shared memory folder is a memory, not code.
    let mut writer = Index::open(&user, None, Access::ReadWrite).await.unwrap();
    writer.index_all(false, &|_| {}).await.unwrap();
    drop(writer);

    // Memories of this workspace, private and shared.
    let dirs = MemoryDirs::of(user.paths(), user.config());
    dirs.save(
        Scope::Private,
        NewMemory {
            title: "Profiles are cached for 5 minutes".into(),
            body: "loadProfile goes through the Redis cache.".into(),
            kind: Kind::Decision,
            ..NewMemory::default()
        },
    )
    .unwrap();

    let index = Index::open(&user, None, Access::ReadOnly).await.unwrap();
    let results = index.search(&query("createOrder")).await.unwrap();
    let code = results.hits.iter().find(|h| h.kind != "memory").expect("a code hit");
    assert_eq!(code.path, "../checkout/src/orders.ts");
    assert_eq!(code.partition, "@checkout");
    let memories: Vec<_> = results.hits.iter().filter(|h| h.kind == "memory").collect();
    assert!(memories.iter().any(|h| h.symbol.as_deref() == Some("Order retries")), "{memories:?}");
    assert!(memories.iter().any(|h| h.symbol.as_deref() == Some("Linked workspace `checkout`")), "{memories:?}");
    assert!(memories.iter().all(|h| h.partition.starts_with("@checkout/memory")), "{memories:?}");

    let results = index.search(&query("profile cache")).await.unwrap();
    let memory = results.hits.iter().find(|h| h.kind == "memory").expect("the decision");
    assert_eq!(memory.partition, "memory");
    assert!(memory.path.starts_with(".brokspace/memory/"), "{}", memory.path);
    assert!(memory.content.contains("Redis"));

    // A new memory shows up without reopening anything.
    dirs.save(
        Scope::Shared,
        NewMemory {
            title: "Webhook signatures".into(),
            body: "Verify stripe signatures.".into(),
            ..NewMemory::default()
        },
    )
    .unwrap();
    let results = index.search(&query("stripe webhook")).await.unwrap();
    let shared = results.hits.iter().find(|h| h.kind == "memory").expect("the shared memory");
    assert!(shared.path.starts_with(".brokspace-memory/"), "{}", shared.path);

    // Without the index.
    let hits = search_memories(&user, "idempotent", 5);
    assert_eq!(hits.first().and_then(|h| h.symbol.as_deref()), Some("Order retries"));

    // Turning memory off removes memories from search.
    let mut config = user.config().clone();
    config.memory.enabled = false;
    let mut user = user;
    user.save_config(config).unwrap();
    let index = Index::open(&user, None, Access::ReadOnly).await.unwrap();
    assert!(index.search(&query("idempotent")).await.unwrap().hits.iter().all(|h| h.kind != "memory"));
}
