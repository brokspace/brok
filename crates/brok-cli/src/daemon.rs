//! Talking to a running `brok run` from other CLI commands.

use anyhow::{Context, Result, bail};
use brok_core::Workspace;
use brok_sdk::{BrokClient, ConnectOptions, Inbox, transport};

/// Whether `brok run` is serving this workspace.
pub async fn is_running(workspace: &Workspace) -> bool {
    transport::is_listening(&workspace.endpoint()).await
}

/// Connects to the running daemon with the operator token.
pub async fn connect(workspace: &Workspace) -> Result<(BrokClient, Inbox)> {
    if !is_running(workspace).await {
        bail!("brok is not running in this workspace. Start it in another terminal with `brok run`");
    }
    let token_file = workspace.paths().operator_token();
    let token = std::fs::read_to_string(&token_file)
        .with_context(|| format!("cannot read {} (is `brok run` running?)", token_file.display()))?;
    let options = ConnectOptions::new("brok-cli", env!("CARGO_PKG_VERSION"), token.trim());
    BrokClient::connect(&workspace.endpoint(), options).await.context("connecting to the brok event bus")
}
