//! The index as a bus participant, run inside `brok run`.
//!
//! - `index:search` requests are answered with `index:search.result`.
//! - `file:changed` events are debounced and applied incrementally; each
//!   update is announced with `index:updated`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use brok_sdk::payload::{FileChanged, IndexUpdated, SearchRequest, SearchResults, topics};
use brok_sdk::{BrokClient, Inbox, TopicPattern};
use tokio::sync::RwLock;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::index::Index;
use crate::search::SearchQuery;

/// Quiet period before a burst of file changes is applied.
const DEBOUNCE: Duration = Duration::from_millis(400);

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
/// read-only (changes are then ignored with a warning).
pub async fn serve(index: Index, writable: bool, client: BrokClient, mut inbox: Inbox, shutdown: CancellationToken) {
    let index = Arc::new(RwLock::new(index));
    if let Err(e) = client.subscribe(subscribes()).await {
        tracing::error!(error = %e, "index service cannot subscribe");
        return;
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
            }
            event = inbox.recv() => {
                let Some(event) = event else { break };
                if let Ok(request) = event.decode::<SearchRequest>() {
                    let index = Arc::clone(&index);
                    let client = client.clone();
                    tokio::spawn(async move {
                        let query = SearchQuery { text: request.query, limit: request.limit, focus: request.focus };
                        let results = match index.read().await.search(&query).await {
                            Ok(results) => results,
                            Err(e) => {
                                tracing::warn!(error = %e, "search failed");
                                SearchResults::default()
                            }
                        };
                        if let Err(e) = client.reply(&event, &results).await {
                            tracing::debug!(error = %e, "cannot deliver search results");
                        }
                    });
                } else if let Ok(change) = event.decode::<FileChanged>() {
                    if !writable {
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

async fn apply(index: &RwLock<Index>, client: &BrokClient, changed: &[String], deleted: &[String]) {
    let result = index.write().await.apply_changes(changed, deleted).await;
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
