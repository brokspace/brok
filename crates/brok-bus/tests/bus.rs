//! End-to-end tests of the bus over the real platform transport.

use std::time::Duration;

use brok_bus::{Bus, Grant};
use brok_sdk::frame::{ErrorCode, ShardContext};
use brok_sdk::payload::{AskChunk, AskRequest, FileChanged, Shutdown};
use brok_sdk::{BrokClient, ClientError, ConnectOptions, Event, Inbox, Topic, TopicPattern};
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(5);

fn p(s: &str) -> TopicPattern {
    TopicPattern::new(s).unwrap()
}

struct Harness {
    bus: Bus,
    shutdown: CancellationToken,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = if cfg!(windows) {
            brok_sdk::Endpoint::new(format!(r"\\.\pipe\brok-bus-test-{}", uuid::Uuid::new_v4().simple()))
        } else {
            brok_sdk::Endpoint::new(dir.path().join("bus.sock").to_string_lossy().into_owned())
        };
        let shutdown = CancellationToken::new();
        let bus = Bus::start(endpoint, shutdown.clone()).unwrap();
        Self { bus, shutdown, _dir: dir }
    }

    async fn connect(&self, grant: Grant) -> (BrokClient, Inbox) {
        let token = self.bus.authenticator().issue(grant);
        BrokClient::connect(self.bus.endpoint(), ConnectOptions::new("test", "0.0.0", token.expose())).await.unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn recv(inbox: &mut Inbox) -> Event {
    tokio::time::timeout(WAIT, inbox.recv()).await.expect("timed out").expect("inbox closed")
}

/// Waits until the router has processed earlier commands from `client`.
async fn barrier(harness: &Harness) {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    let _ = harness.bus.stats().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn routes_by_subscription_and_stamps_source() {
    let h = Harness::start();
    let (watcher, mut watcher_inbox) = h.connect(Grant::service("watcher", vec![], vec![p("file:*")])).await;
    let (_bystander, mut bystander_inbox) = h.connect(Grant::service("bystander", vec![], vec![p("agent:*")])).await;
    let (tracker, _) = h.connect(Grant::service("tracker", vec![p("file:changed")], vec![])).await;

    watcher.subscribe([p("file:*")]).await.unwrap();
    barrier(&h).await;

    let mut event = Event::from_payload(&FileChanged { changed: vec!["a.rs".into()], deleted: vec![] }).unwrap();
    event.source = "spoofed".into();
    tracker.publish(event).await.unwrap();

    let received = recv(&mut watcher_inbox).await;
    assert_eq!(received.source, "tracker");
    assert_eq!(received.decode::<FileChanged>().unwrap().changed, vec!["a.rs"]);
    assert!(tokio::time::timeout(Duration::from_millis(200), bystander_inbox.recv()).await.is_err());
}

#[tokio::test]
async fn enforces_publish_permissions() {
    let h = Harness::start();
    let (client, _) = h.connect(Grant::service("limited", vec![p("file:changed")], vec![])).await;

    let forbidden = Event::new(Topic::new("security:alert").unwrap(), serde_json::Value::Null);
    let mut replies = client.request(forbidden).await.unwrap();
    let err = replies.next_timeout(WAIT).await.unwrap_err();
    assert!(matches!(err, ClientError::Rejected { code: ErrorCode::Forbidden, .. }), "{err:?}");

    let system = Event::new(Topic::new("system:shutdown").unwrap(), serde_json::Value::Null);
    let mut replies = client.request(system).await.unwrap();
    assert!(replies.next_timeout(WAIT).await.is_err());
    assert_eq!(h.bus.stats().await.denied, 2);
}

#[tokio::test]
async fn replies_only_reach_the_requester() {
    let h = Harness::start();
    let (agent, mut agent_inbox) =
        h.connect(Grant::service("agent", vec![p("agent:ask.*")], vec![p("agent:ask")])).await;
    let (snoop, mut snoop_inbox) = h.connect(Grant::service("snoop", vec![], vec![p("agent:*")])).await;
    let (cli, _) = h.connect(Grant::operator(ShardContext::default())).await;

    agent.subscribe([p("agent:ask")]).await.unwrap();
    snoop.subscribe([p("agent:*")]).await.unwrap();
    barrier(&h).await;

    let ask = AskRequest { prompt: "why?".into(), provider: None, context_chunks: None, skill: None };
    let mut replies = cli.request(Event::from_payload(&ask).unwrap()).await.unwrap();

    let request = recv(&mut agent_inbox).await;
    assert_eq!(recv(&mut snoop_inbox).await.id, request.id, "requests are broadcast normally");
    agent.reply(&request, &AskChunk { text: "because".into() }).await.unwrap();

    let reply = replies.next_timeout(WAIT).await.unwrap();
    assert_eq!(reply.decode::<AskChunk>().unwrap().text, "because");
    assert_eq!(reply.source, "agent");
    assert!(tokio::time::timeout(Duration::from_millis(200), snoop_inbox.recv()).await.is_err());
}

#[tokio::test]
async fn subscriptions_outside_the_grant_are_ignored() {
    let h = Harness::start();
    let (client, mut inbox) = h.connect(Grant::service("narrow", vec![], vec![p("file:changed")])).await;
    let (publisher, _) = h.connect(Grant::operator(ShardContext::default())).await;

    client.subscribe([p("*")]).await.unwrap();
    barrier(&h).await;
    publisher.emit(&FileChanged::default()).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(200), inbox.recv()).await.is_err());
}

#[tokio::test]
async fn system_events_reach_everyone() {
    let h = Harness::start();
    let (_a, mut a) = h.connect(Grant::service("a", vec![], vec![])).await;
    let (_b, mut b) = h.connect(Grant::service("b", vec![], vec![])).await;
    barrier(&h).await;

    h.bus.emit(&Shutdown { reason: "test".into() }).await.unwrap();
    for inbox in [&mut a, &mut b] {
        let event = recv(inbox).await;
        assert_eq!(event.source, brok_bus::BUS_IDENTITY);
        assert_eq!(event.decode::<Shutdown>().unwrap().reason, "test");
    }
}

#[tokio::test]
async fn invalid_tokens_are_rejected() {
    let h = Harness::start();
    let err =
        BrokClient::connect(h.bus.endpoint(), ConnectOptions::new("intruder", "0", "not-a-token")).await.unwrap_err();
    assert!(matches!(err, ClientError::Rejected { code: ErrorCode::Unauthorized, .. }), "{err:?}");
}
