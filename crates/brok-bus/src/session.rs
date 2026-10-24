//! One client session: handshake, then a read loop feeding the router and
//! a writer task draining the session's outbound queue.

use std::sync::Arc;
use std::time::Duration;

use brok_sdk::PROTOCOL_VERSION;
use brok_sdk::codec::ServerCodec;
use brok_sdk::frame::{ClientFrame, ErrorCode, ServerFrame, Welcome};
use brok_sdk::transport::BoxedStream;
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::auth::Authenticator;
use crate::router::{Command, ConnId, RouterHandle};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Frames buffered per client before the router starts dropping events.
const OUTBOUND_CAPACITY: usize = 1024;

pub(crate) async fn serve(
    stream: BoxedStream,
    conn: ConnId,
    auth: Arc<Authenticator>,
    router: RouterHandle,
    shutdown: CancellationToken,
) {
    let mut framed = Framed::new(stream, ServerCodec::new());

    let hello = match tokio::time::timeout(HANDSHAKE_TIMEOUT, framed.next()).await {
        Ok(Some(Ok(ClientFrame::Hello(hello)))) => hello,
        Ok(Some(Ok(_))) => return reject(&mut framed, ErrorCode::InvalidFrame, "expected a hello frame").await,
        Ok(Some(Err(e))) => return reject(&mut framed, ErrorCode::InvalidFrame, &e.to_string()).await,
        Ok(None) => return,
        Err(_) => return reject(&mut framed, ErrorCode::InvalidFrame, "handshake timed out").await,
    };
    if hello.protocol != PROTOCOL_VERSION {
        let message = format!("client speaks protocol {}, bus speaks {PROTOCOL_VERSION}", hello.protocol);
        return reject(&mut framed, ErrorCode::ProtocolMismatch, &message).await;
    }
    let Some(grant) = auth.authenticate(&hello.token) else {
        tracing::warn!(client = %hello.name, "rejected connection with an invalid token");
        return reject(&mut framed, ErrorCode::Unauthorized, "invalid session token").await;
    };

    let welcome = Welcome {
        protocol: PROTOCOL_VERSION,
        session_id: Uuid::new_v4(),
        identity: grant.identity.clone(),
        context: grant.context.clone(),
    };
    if framed.send(ServerFrame::Welcome(welcome)).await.is_err() {
        return;
    }
    tracing::info!(identity = %grant.identity, client = %hello.name, version = %hello.version, "client connected");

    let (mut sink, mut frames) = framed.split();
    let (tx, mut rx) = mpsc::channel::<ServerFrame>(OUTBOUND_CAPACITY);
    let identity = grant.identity.clone();
    router.send(Command::Join { conn, grant, tx: tx.clone() }).await;

    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if sink.send(frame).await.is_err() {
                break;
            }
        }
    });

    loop {
        let frame = tokio::select! {
            () = shutdown.cancelled() => break,
            frame = frames.next() => frame,
        };
        match frame {
            Some(Ok(ClientFrame::Publish { event })) => router.send(Command::Publish { conn: Some(conn), event }).await,
            Some(Ok(ClientFrame::Subscribe { patterns })) => router.send(Command::Subscribe { conn, patterns }).await,
            Some(Ok(ClientFrame::Hello(_))) => {
                let frame = ServerFrame::Error {
                    code: ErrorCode::InvalidFrame,
                    message: "already authenticated".into(),
                    correlation_id: None,
                };
                let _ = tx.try_send(frame);
            }
            Some(Err(e)) => {
                tracing::warn!(%identity, error = %e, "closing session after a malformed frame");
                let frame =
                    ServerFrame::Error { code: ErrorCode::InvalidFrame, message: e.to_string(), correlation_id: None };
                let _ = tx.try_send(frame);
                break;
            }
            None => break,
        }
    }

    router.send(Command::Leave { conn }).await;
    drop(tx);
    // Let queued frames (e.g. a final error) flush, but never hang on a dead peer.
    let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
    tracing::info!(%identity, "client disconnected");
}

async fn reject(framed: &mut Framed<BoxedStream, ServerCodec>, code: ErrorCode, message: &str) {
    let _ = framed.send(ServerFrame::Error { code, message: message.into(), correlation_id: None }).await;
}
