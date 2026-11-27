//! The index as a bus participant, run inside `brok run`.
//!
//! - `index:search` requests are answered with `index:search.result`.
//! - `file:changed` events are debounced and applied incrementally; each
//!   update is announced with `index:updated`.
//! - Changes are searchable by keywords first. Vectors are computed in the
//!   background of the service, without holding the index while embedding.
//! - While another process writes the index (`brok index`), the service
//!   serves what is saved and takes over when that process ends.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use brok_sdk::payload::{FileChanged, IndexUpdated, SearchRequest, SearchResults, SearchScope, topics};
use brok_sdk::{BrokClient, Inbox, TopicPattern};
use tokio::sync::{Notify, RwLock};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::index::{Access, Index};
use crate::search::SearchQuery;

/// Quiet period before a burst of file changes is applied.
const DEBOUNCE: Duration = Duration::from_millis(400);

/// Widens a workspace's own results with those of its Meta Workspace members:
/// given the query text, the limit and the workspace's results, it returns
/// the merged results. Supplied by `brok run`, which knows the members; the
/// index itself knows only its own workspace.
pub type Widen =
    Arc<dyn Fn(String, usize, SearchResults) -> Pin<Box<dyn Future<Output = SearchResults> + Send>> + Send + Sync>;

/// How often a read-only service looks whether it can write again.
const RETRY: Duration = Duration::from_secs(3);

/// Opens the workspace's index again, for a service that could not write it.
pub type Reopen = Arc<dyn Fn(Access) -> Pin<Box<dyn Future<Output = anyhow::Result<Index>> + Send>> + Send + Sync>;

/// Where the reopening looks for saved changes, and how to open the index.
pub struct Takeover {
    /// The workspace layout.
    pub paths: brok_core::WorkspacePaths,
    /// Opens the index.
    pub reopen: Reopen,
}

/// Topics the index service publishes.
pub fn publishes() -> Vec<TopicPattern> {
    vec![TopicPattern::new("index:*").expect("valid pattern")]
}

/// Topics the index service subscribes to.
pub fn subscribes() -> Vec<TopicPattern> {
    [topics::FILE_CHANGED, topics::INDEX_SEARCH].map(|t| TopicPattern::new(t).expect("valid pattern")).to_vec()
}

/// Serves the index on the bus until `shutdown` fires or the connection
/// closes. `writable` is false when the index could only be opened
/// read-only (changes are then ignored with a warning, until `takeover`
/// manages to open it for writing). `widen` answers
/// requests whose scope is the whole Meta Workspace; without it they get this
/// workspace's results.
pub async fn serve(
    index: Index,
    writable: bool,
    takeover: Option<Takeover>,
    widen: Option<Widen>,
    client: BrokClient,
    mut inbox: Inbox,
    shutdown: CancellationToken,
) {
    let index = Arc::new(RwLock::new(index));
    if let Err(e) = client.subscribe(subscribes()).await {
        tracing::error!(error = %e, "index service cannot subscribe");
        return;
    }
    let writable = Arc::new(AtomicBool::new(writable));
    let waiting = Arc::new(Notify::new());
    if writable.load(Ordering::SeqCst) {
        catch_up(&index, &client).await;
        waiting.notify_one();
    }
    // Both end with this function.
    let mut background = tokio::task::JoinSet::new();
    background.spawn(fill_vectors(Arc::clone(&index), Arc::clone(&waiting)));
    if let Some(takeover) = takeover.filter(|_| !writable.load(Ordering::SeqCst)) {
        let (index, client) = (Arc::clone(&index), client.clone());
        let (writable, waiting) = (Arc::clone(&writable), Arc::clone(&waiting));
        background.spawn(async move {
            take_over(takeover, &index).await;
            writable.store(true, Ordering::SeqCst);
            catch_up(&index, &client).await;
            waiting.notify_one();
        });
    }

    let mut changed = BTreeSet::new();
    let mut deleted = BTreeSet::new();
    let mut deadline: Option<Instant> = None;

    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = sleep_until(deadline) => {
                deadline = None;
                let (changed, deleted): (Vec<String>, Vec<String>) =
                    (std::mem::take(&mut changed).into_iter().collect(), std::mem::take(&mut deleted).into_iter().collect());
                apply(&index, &client, &changed, &deleted).await;
                waiting.notify_one();
            }
            event = inbox.recv() => {
                let Some(event) = event else { break };
                if let Ok(request) = event.decode::<SearchRequest>() {
                    let index = Arc::clone(&index);
                    let client = client.clone();
                    let widen = widen.clone();
                    tokio::spawn(async move {
                        let (text, limit, scope) = (request.query.clone(), request.limit, request.scope);
                        let query = SearchQuery { text: request.query, limit: request.limit, focus: request.focus };
                        let results = match index.read().await.search(&query).await {
                            Ok(results) => results,
                            Err(e) => {
                                tracing::warn!(error = %e, "search failed");
                                SearchResults::default()
                            }
                        };
                        let results = match (scope, widen) {
                            (SearchScope::Workspace, Some(widen)) => widen(text, limit, results).await,
                            _ => results,
                        };
                        if let Err(e) = client.reply(&event, &results).await {
                            tracing::debug!(error = %e, "cannot deliver search results");
                        }
                    });
                } else if let Ok(change) = event.decode::<FileChanged>() {
                    if !writable.load(Ordering::SeqCst) {
                        tracing::warn!("index is read-only in this session; ignoring file changes");
                        continue;
                    }
                    for path in change.changed {
                        deleted.remove(&path);
                        changed.insert(path);
                    }
                    for path in change.deleted {
                        changed.remove(&path);
                        deleted.insert(path);
                    }
                    deadline = Some(Instant::now() + DEBOUNCE);
                }
            }
        }
    }
}

/// Embeds waiting chunks whenever told there may be some. The index is only
/// locked to pick the files and to save a result, so searches go on.
async fn fill_vectors(index: Arc<RwLock<Index>>, waiting: Arc<Notify>) {
    loop {
        waiting.notified().await;
        let mut pace = index.read().await.pace();
        let mut saved_any = false;
        loop {
            let job = index.read().await.vector_job(pace.chunks);
            let Some(job) = job else { break };
            let started = Instant::now();
            let batch = match job.run().await {
                Ok(batch) => batch,
                Err(e) => {
                    tracing::error!(error = %e, "embedding failed; trying again after the next change");
                    break;
                }
            };
            let embedding = started.elapsed();
            let saving = Instant::now();
            match index.write().await.save_vectors(batch).await {
                Ok(saved) => {
                    saved_any = true;
                    pace.record(saved, embedding, saving.elapsed());
                }
                Err(e) => {
                    tracing::error!(error = %e, "saving vectors failed; trying again after the next change");
                    break;
                }
            }
        }
        if saved_any {
            let mut index = index.write().await;
            index.prune_replaced().await;
            tracing::info!(waiting = index.waiting_chunks(), "vectors saved");
        }
    }
}

/// Waits until the index can be opened for writing, showing readers what the
/// writing process saves meanwhile.
async fn take_over(takeover: Takeover, index: &RwLock<Index>) {
    let mut seen = Index::saved_at(&takeover.paths);
    loop {
        tokio::time::sleep(RETRY).await;
        match (takeover.reopen)(Access::ReadWrite).await {
            Ok(writer) => {
                *index.write().await = writer;
                tracing::info!("the index can be written again");
                return;
            }
            Err(e) => tracing::debug!(error = %e, "the index is still read only"),
        }
        let saved = Index::saved_at(&takeover.paths);
        if saved != seen
            && let Ok(reader) = (takeover.reopen)(Access::ReadOnly).await
        {
            *index.write().await = reader;
            seen = saved;
        }
    }
}

/// Indexes whatever changed while no daemon was running (pulls, checkouts,
/// edits). Incremental, so it is quick when nothing changed. Searches wait
/// for its text pass only.
async fn catch_up(index: &RwLock<Index>, client: &BrokClient) {
    let result = index.write().await.index_text(false, &|_| {}).await;
    match result {
        Ok(report) => {
            if report.files_indexed + report.files_removed > 0 {
                tracing::info!(
                    indexed = report.files_indexed,
                    removed = report.files_removed,
                    ms = report.duration.as_millis(),
                    "index caught up"
                );
                let _ = client.emit(&IndexUpdated::from(&report)).await;
            }
        }
        Err(e) => tracing::error!(error = %e, "catching up the index failed"),
    }
}

async fn apply(index: &RwLock<Index>, client: &BrokClient, changed: &[String], deleted: &[String]) {
    let result = index.write().await.apply_changes_text(changed, deleted).await;
    match result {
        Ok(report) => {
            tracing::info!(
                indexed = report.files_indexed,
                removed = report.files_removed,
                chunks = report.chunks,
                ms = report.duration.as_millis(),
                "index updated"
            );
            if let Err(e) = client.emit(&IndexUpdated::from(&report)).await {
                tracing::debug!(error = %e, "cannot announce index update");
            }
        }
        Err(e) => tracing::error!(error = %e, "failed to apply file changes"),
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
