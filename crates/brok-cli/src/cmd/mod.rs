//! One module per subcommand.

pub mod ask;
pub mod bus;
pub mod chat;
pub mod doctor;
pub mod git_assist;
pub mod index;
pub mod init;
pub mod mcp;
pub mod model;
pub mod run;
pub mod search;
pub mod service;
pub mod setup;
pub mod shard;
pub mod skill;
pub mod status;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use brok_core::Workspace;
use brok_index::Embedder;

/// Opens the workspace containing `cwd`.
pub fn workspace(cwd: &Path) -> Result<Workspace> {
    Ok(Workspace::discover(cwd)?)
}

/// Loads the embedder for search: failures degrade to BM25 with a warning.
pub fn embedder_for_search(workspace: &Workspace) -> Option<Arc<dyn Embedder>> {
    match brok_index::load_embedder(&workspace.config().embeddings, &workspace.paths().models) {
        Ok(embedder) => embedder,
        Err(e) => {
            crate::ui::warn(&format!("semantic search unavailable, using text search only: {e:#}"));
            None
        }
    }
}

/// The partition-relative focus of `cwd`, used to prioritise hot storage.
pub fn focus_of(workspace: &Workspace, cwd: &Path) -> Option<String> {
    let cwd = brok_core::workspace::dunce_canonicalize(cwd).ok()?;
    brok_core::fsutil::to_workspace_relative(workspace.root(), &cwd).filter(|rel| !rel.is_empty())
}
