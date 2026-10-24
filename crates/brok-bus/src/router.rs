//! The routing actor.
//!
//! A single task owns all routing state (connections, subscriptions, the
//! reply table) and processes commands sequentially, so there are no locks
//! on the hot path. Delivery uses `try_send`: a slow consumer loses events
//! (and is told so in the logs) instead of stalling the whole bus.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use brok_sdk::frame::{ErrorCode, ServerFrame};
use brok_sdk::topic::SYSTEM_NAMESPACE;
use brok_sdk::{Event, TopicPattern};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::auth::Grant;

/// Connection identifier, unique for the lifetime of the bus.
pub(crate) type ConnId = u64;

/// Identity stamped on events published by the bus itself.
pub const BUS_IDENTITY: &str = "brok";

const REPLY_TABLE_CAPACITY: usize = 65_536;
const REPLY_TTL: Duration = Duration::from_secs(15 * 60);

pub(crate) enum Command {
    Join { conn: ConnId, grant: Arc<Grant>, tx: mpsc::Sender<ServerFrame> },
    Leave { conn: ConnId },
    Subscribe { conn: ConnId, patterns: Vec<TopicPattern> },
    Publish { conn: Option<ConnId>, event: Event },
    Stats { reply: oneshot::Sender<BusStats> },
}

/// Runtime counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BusStats {
    /// Open sessions.
    pub connections: usize,
    /// Events accepted for routing.
    pub published: u64,
    /// Event deliveries to clients.
    pub delivered: u64,
    /// Deliveries dropped because a client was too slow.
    pub dropped: u64,
    /// Publications rejected by the ACL.
    pub denied: u64,
}

/// Cheap, cloneable handle to the router task.
#[derive(Clone, Debug)]
pub(crate) struct RouterHandle {
    tx: mpsc::Sender<Command>,
}

impl RouterHandle {
    pub(crate) fn spawn() -> Self {
        let (tx, rx) = mpsc::channel(1024);
        tokio::spawn(Router::default().run(rx));
        Self { tx }
    }

    pub(crate) async fn send(&self, command: Command) {
        // The router only stops when every handle is gone.
        let _ = self.tx.send(command).await;
    }

    pub(crate) async fn stats(&self) -> BusStats {
        let (reply, rx) = oneshot::channel();
        self.send(Command::Stats { reply }).await;
        rx.await.unwrap_or_default()
    }
}

struct Connection {
    grant: Arc<Grant>,
    tx: mpsc::Sender<ServerFrame>,
    subscriptions: Vec<TopicPattern>,
}

/// Remembers who published which event, so replies go back to the
/// requester only. Bounded in size and age.
#[derive(Default)]
struct ReplyTable {
    origin: HashMap<Uuid, ConnId>,
    order: VecDeque<(Uuid, Instant)>,
}

impl ReplyTable {
    fn record(&mut self, event: Uuid, conn: ConnId) {
        let now = Instant::now();
        while let Some(&(id, at)) = self.order.front() {
            if self.order.len() < REPLY_TABLE_CAPACITY && now.duration_since(at) < REPLY_TTL {
                break;
            }
            self.order.pop_front();
            self.origin.remove(&id);
        }
        self.origin.insert(event, conn);
        self.order.push_back((event, now));
    }

    fn origin_of(&self, event: &Uuid) -> Option<ConnId> {
        self.origin.get(event).copied()
    }
}

#[derive(Default)]
struct Router {
    connections: HashMap<ConnId, Connection>,
    replies: ReplyTable,
    stats: BusStats,
}

impl Router {
    async fn run(mut self, mut rx: mpsc::Receiver<Command>) {
        while let Some(command) = rx.recv().await {
            self.handle(command);
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Join { conn, grant, tx } => {
                tracing::debug!(conn, identity = %grant.identity, "session joined");
                self.connections.insert(conn, Connection { grant, tx, subscriptions: Vec::new() });
            }
            Command::Leave { conn } => {
                if let Some(c) = self.connections.remove(&conn) {
                    tracing::debug!(conn, identity = %c.grant.identity, "session left");
                }
            }
            Command::Subscribe { conn, patterns } => self.subscribe(conn, patterns),
            Command::Publish { conn: Some(conn), event } => self.publish_from(conn, event),
            Command::Publish { conn: None, mut event } => {
                event.source = BUS_IDENTITY.into();
                self.deliver(None, event);
            }
            Command::Stats { reply } => {
                let _ = reply.send(BusStats { connections: self.connections.len(), ..self.stats });
            }
        }
    }

    fn subscribe(&mut self, conn: ConnId, patterns: Vec<TopicPattern>) {
        let Some(connection) = self.connections.get_mut(&conn) else { return };
        let (allowed, denied): (Vec<_>, Vec<_>) =
            patterns.into_iter().partition(|pattern| connection.grant.can_subscribe(pattern));
        for pattern in allowed {
            if !connection.subscriptions.contains(&pattern) {
                connection.subscriptions.push(pattern);
            }
        }
        if !denied.is_empty() {
            let list = denied.iter().map(TopicPattern::as_str).collect::<Vec<_>>().join(", ");
            let message = format!("`{}` may not subscribe to: {list}", connection.grant.identity);
            tracing::warn!("{message}");
            self.send(conn, ServerFrame::Error { code: ErrorCode::Forbidden, message, correlation_id: None });
        }
    }

    fn publish_from(&mut self, conn: ConnId, mut event: Event) {
        let Some(connection) = self.connections.get(&conn) else { return };
        event.source.clone_from(&connection.grant.identity);
        if !connection.grant.can_publish(&event.topic) {
            self.stats.denied += 1;
            let message = format!("`{}` may not publish to `{}`", event.source, event.topic);
            tracing::warn!("{message}");
            let frame = ServerFrame::Error { code: ErrorCode::Forbidden, message, correlation_id: Some(event.id) };
            self.send(conn, frame);
            return;
        }
        self.deliver(Some(conn), event);
    }

    fn deliver(&mut self, origin: Option<ConnId>, event: Event) {
        self.stats.published += 1;

        // Replies go straight back to the requester, nobody else sees them.
        if let Some(target) = event.correlation_id.and_then(|id| self.replies.origin_of(&id)) {
            self.send(target, ServerFrame::Event { event });
            return;
        }
        if let Some(origin) = origin {
            self.replies.record(event.id, origin);
        }

        let broadcast = event.topic.namespace() == SYSTEM_NAMESPACE;
        let targets: Vec<ConnId> = self
            .connections
            .iter()
            .filter(|(id, _)| Some(**id) != origin)
            .filter(|(_, c)| broadcast || c.subscriptions.iter().any(|p| p.matches(&event.topic)))
            .map(|(id, _)| *id)
            .collect();
        for target in targets {
            self.send(target, ServerFrame::Event { event: event.clone() });
        }
    }

    fn send(&mut self, conn: ConnId, frame: ServerFrame) {
        let Some(connection) = self.connections.get(&conn) else { return };
        match connection.tx.try_send(frame) {
            Ok(()) => self.stats.delivered += 1,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.stats.dropped += 1;
                tracing::warn!(identity = %connection.grant.identity, "client is too slow; event dropped");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.connections.remove(&conn);
            }
        }
    }
}
