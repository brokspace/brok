//! The Brokspace event bus.
//!
//! An asynchronous, local-only pub/sub broker. Clients (shards, the CLI,
//! built-in services) connect over a Unix domain socket or a Windows named
//! pipe, authenticate with a session token and exchange
//! [`brok_sdk::Event`]s. Capabilities declared in each shard manifest are
//! enforced here:
//!
//! - publishing outside `publishes_to` is rejected with `Forbidden`;
//! - subscribing outside `subscribes_to` is rejected with `Forbidden`;
//! - `source` is overwritten with the authenticated identity (no spoofing);
//! - `system:*` can only be published by the bus and reaches everyone;
//! - replies (events carrying a `correlation_id`) are delivered only to the
//!   client that published the original request.

#![warn(missing_docs)]

pub mod auth;
mod router;
mod session;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use brok_sdk::transport::{Endpoint, Listener};
use brok_sdk::{Event, Payload};
use tokio_util::sync::CancellationToken;

pub use auth::{Authenticator, Grant, Role, SessionToken};
pub use router::{BUS_IDENTITY, BusStats};

use router::{Command, RouterHandle};

/// A running bus. Dropping it does not stop the bus; cancel the shutdown
/// token passed to [`Bus::start`] instead.
#[derive(Debug, Clone)]
pub struct Bus {
    endpoint: Endpoint,
    auth: Arc<Authenticator>,
    router: RouterHandle,
}

impl Bus {
    /// Binds `endpoint` and starts accepting clients until `shutdown` fires.
    pub fn start(endpoint: Endpoint, shutdown: CancellationToken) -> io::Result<Self> {
        let mut listener = Listener::bind(&endpoint)?;
        let auth = Arc::new(Authenticator::new());
        let router = RouterHandle::spawn();

        let accept_auth = Arc::clone(&auth);
        let accept_router = router.clone();
        tokio::spawn(async move {
            let next_conn = AtomicU64::new(1);
            loop {
                let accepted = tokio::select! {
                    () = shutdown.cancelled() => break,
                    accepted = listener.accept() => accepted,
                };
                match accepted {
                    Ok(stream) => {
                        let conn = next_conn.fetch_add(1, Ordering::Relaxed);
                        tokio::spawn(session::serve(
                            stream,
                            conn,
                            Arc::clone(&accept_auth),
                            accept_router.clone(),
                            shutdown.clone(),
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to accept a client");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
            tracing::debug!("bus listener stopped");
        });

        tracing::info!(%endpoint, "event bus listening");
        Ok(Self { endpoint, auth, router })
    }

    /// The endpoint clients connect to.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Token table, used to issue credentials for shards and the CLI.
    pub fn authenticator(&self) -> &Arc<Authenticator> {
        &self.auth
    }

    /// Publishes an event as the bus itself (`source = "brok"`). Events in
    /// the `system:` namespace reach every client.
    pub async fn publish(&self, event: Event) {
        self.router.send(Command::Publish { conn: None, event }).await;
    }

    /// Typed variant of [`Bus::publish`].
    pub async fn emit<P: Payload>(&self, payload: &P) -> Result<(), serde_json::Error> {
        self.publish(Event::from_payload(payload)?).await;
        Ok(())
    }

    /// Current counters.
    pub async fn stats(&self) -> BusStats {
        self.router.stats().await
    }
}
