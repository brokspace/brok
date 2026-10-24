//! `brok run`: the orchestrator.
//!
//! Startup order: event bus → operator token for the CLI → index service →
//! shards. Shutdown (Ctrl+C) runs in reverse: `system:shutdown` is broadcast,
//! shards get a grace period, then the bus stops and runtime files are
//! removed.

use std::path::Path;

use anyhow::{Context, Result, bail};
use brok_bus::{Bus, Grant};
use brok_core::Workspace;
use brok_index::{Access, Index};
use brok_sdk::frame::ShardContext;
use brok_sdk::payload::Shutdown;
use brok_sdk::{BrokClient, ConnectOptions, transport};
use brok_shard::{LaunchSpec, Registry, SandboxEnv, supervise};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::ui;

/// Arguments of `brok run`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Do not start any shard (bus and index only).
    #[arg(long)]
    no_shards: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let endpoint = workspace.endpoint();
    if transport::is_listening(&endpoint).await {
        bail!("brok is already running for {}", workspace.root().display());
    }

    let bus_shutdown = CancellationToken::new();
    let bus = Bus::start(endpoint.clone(), bus_shutdown.clone()).context("starting the event bus")?;
    let paths = workspace.paths().clone();
    let base_context = ShardContext {
        workspace_root: workspace.root().to_path_buf(),
        settings: serde_json::Value::Null,
        llm: serde_json::to_value(&workspace.config().llm)?,
    };

    // CLI access.
    let operator = bus.authenticator().issue(Grant::operator(base_context.clone()));
    write_secret(&paths.operator_token(), operator.expose())?;
    brok_core::fsutil::write_atomic(&paths.pid_file(), std::process::id().to_string().as_bytes())?;

    // Index service.
    let index_shutdown = CancellationToken::new();
    let mut tasks = JoinSet::new();
    start_index_service(&workspace, &bus, &index_shutdown, &mut tasks).await;

    // Shards.
    let shard_shutdown = CancellationToken::new();
    let mut shards = JoinSet::new();
    let mut started = Vec::new();
    if !args.no_shards {
        for shard in Registry::new(&paths).list()? {
            let name = shard.manifest.name.clone();
            if !workspace.config().shard_enabled(&name) {
                tracing::info!(shard = %name, "shard disabled in brok.yml");
                continue;
            }
            let Some(executable) = shard.executable() else { continue }; // skill-only shard
            if !executable.is_file() {
                ui::warn(&format!("{name}: executable missing at {}", executable.display()));
                continue;
            }
            let context = ShardContext { settings: workspace.config().shard_settings(&name), ..base_context.clone() };
            let token = bus.authenticator().issue(Grant::for_shard(&shard.manifest, context));
            let env = SandboxEnv::new(&shard.manifest, workspace.root(), &endpoint, token.expose());
            let spec = LaunchSpec { shard, env, cwd: workspace.root().to_path_buf(), log_dir: paths.logs.clone() };
            shards.spawn(supervise(spec, shard_shutdown.clone()));
            started.push(name);
        }
    }

    println!();
    ui::success(&format!("brok is running for {}", workspace.root().display()));
    ui::hint(&format!("bus:    {endpoint}"));
    ui::hint(&format!("shards: {}", if started.is_empty() { "none".to_owned() } else { started.join(", ") }));
    ui::hint("Press Ctrl+C to stop.");
    println!();

    tokio::signal::ctrl_c().await.context("waiting for Ctrl+C")?;
    println!();
    ui::step("Shutting down…");

    let _ = bus.emit(&Shutdown { reason: "brok run is stopping".into() }).await;
    shard_shutdown.cancel();
    let grace = brok_shard::supervisor::GRACE_PERIOD + std::time::Duration::from_secs(2);
    let _ = tokio::time::timeout(grace, async { while shards.join_next().await.is_some() {} }).await;
    index_shutdown.cancel();
    let _ =
        tokio::time::timeout(std::time::Duration::from_secs(5), async { while tasks.join_next().await.is_some() {} })
            .await;
    bus_shutdown.cancel();

    let _ = std::fs::remove_file(paths.operator_token());
    let _ = std::fs::remove_file(paths.pid_file());
    ui::success("Stopped");
    Ok(())
}

/// Opens the index for writing (falling back to read-only search when the
/// model or the write lock is unavailable) and serves it on the bus.
async fn start_index_service(workspace: &Workspace, bus: &Bus, shutdown: &CancellationToken, tasks: &mut JoinSet<()>) {
    let embedder = match brok_index::load_embedder(&workspace.config().embeddings, &workspace.paths().models) {
        Ok(embedder) => embedder,
        Err(e) => {
            ui::warn(&format!("{e:#}"));
            None
        }
    };
    let (index, writable) = match Index::open(workspace, embedder.clone(), Access::ReadWrite).await {
        Ok(index) => (index, true),
        Err(e) => {
            ui::warn(&format!("index is read-only in this session: {e:#}"));
            match Index::open(workspace, embedder, Access::ReadOnly).await {
                Ok(index) => (index, false),
                Err(e) => {
                    ui::error(&format!("index unavailable: {e:#}"));
                    return;
                }
            }
        }
    };

    let grant = Grant::service("brok-index", brok_index::service::publishes(), brok_index::service::subscribes());
    let token = bus.authenticator().issue(grant);
    let options = ConnectOptions::new("brok-index", env!("CARGO_PKG_VERSION"), token.expose());
    match BrokClient::connect(bus.endpoint(), options).await {
        Ok((client, inbox)) => {
            tasks.spawn(brok_index::service::serve(index, writable, client, inbox, shutdown.clone()));
        }
        Err(e) => ui::error(&format!("index service cannot join the bus: {e:#}")),
    }
}

/// Writes a secret readable only by the current user.
fn write_secret(path: &Path, secret: &str) -> Result<()> {
    brok_core::fsutil::write_atomic(path, secret.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
