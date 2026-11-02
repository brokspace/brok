//! The indexes of linked workspaces (`workspace.links`), searched with the
//! workspace's own.
//!
//! Each one is opened read only on the first search and reopened when its
//! partitions are saved again (their manifests change), so results follow
//! the linked workspace's own daemon or `brok link sync`.

use std::path::Path;
use std::time::UNIX_EPOCH;

use brok_core::links::{LinkIndex, LinkKind};
use brok_core::{BrokConfig, Link, Workspace};
use serde::Serialize;

use crate::index::{Access, Index};
use crate::partition_index::MANIFEST_FILE;

/// A linked workspace and, once opened, its index.
pub(crate) struct LinkedIndex {
    pub(crate) link: Link,
    /// Prefix of its paths in search results (`../checkout/`).
    pub(crate) prefix: String,
    pub(crate) index: Option<Index>,
    stamp: Option<u128>,
    pub(crate) error: Option<String>,
}

/// `brok status` / `brok link list` view of a link.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkStatus {
    /// Name in `workspace.links`.
    pub name: String,
    /// `local` or `git`.
    pub kind: LinkKind,
    /// The sources, relative to this workspace when possible.
    pub root: String,
    /// Absolute directory of the sources.
    pub path: String,
    /// The git URL of a git link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// Its branch, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What it is, as configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether its memories are searched.
    pub memory: bool,
    /// `own` (a Brokspace workspace) or `managed` (indexed by this one).
    pub index: LinkIndex,
    /// Whether its sources are on disk.
    pub exists: bool,
    /// Indexed files.
    pub files: usize,
    /// Stored chunks.
    pub chunks: usize,
    /// Why it cannot be searched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl LinkedIndex {
    /// Every link of `workspace`, not opened yet.
    pub(crate) fn all(workspace: &Workspace) -> Vec<Self> {
        Link::all(workspace)
            .into_iter()
            .map(|link| Self { prefix: prefix(&link, workspace), link, index: None, stamp: None, error: None })
            .collect()
    }

    /// Opens the index, or reopens it when it changed on disk.
    pub(crate) async fn refresh(&mut self, parent: &BrokConfig) {
        let workspace = match self.link.workspace(parent) {
            Ok(workspace) => workspace,
            Err(e) => {
                self.close(Some(e.to_string()));
                return;
            }
        };
        let Some(stamp) = stamp(&workspace.paths().databases) else {
            let hint = match self.link.index() {
                LinkIndex::Own => format!("not indexed yet (run `brok index` in {})", self.link.root.display()),
                LinkIndex::Managed => format!("not indexed yet (run `brok link sync {}`)", self.link.name),
            };
            self.close(Some(hint));
            return;
        };
        if self.index.is_some() && self.stamp == Some(stamp) {
            return;
        }
        // Read only and without an embedder: the query vector comes from
        // the parent, and is used only where the model matches.
        match Index::open(&workspace, None, Access::ReadOnly).await {
            Ok(index) => {
                self.index = Some(index);
                self.stamp = Some(stamp);
                self.error = None;
            }
            Err(e) => self.close(Some(format!("{e:#}"))),
        }
    }

    fn close(&mut self, error: Option<String>) {
        self.index = None;
        self.stamp = None;
        self.error = error;
    }

    /// Current state, for status output.
    pub(crate) async fn status(&mut self, parent: &BrokConfig) -> LinkStatus {
        self.refresh(parent).await;
        let (mut files, mut chunks) = (0, 0);
        if let Some(index) = &self.index {
            match index.status().await {
                Ok(parts) => {
                    files = parts.iter().map(|p| p.files).sum();
                    chunks = parts.iter().map(|p| p.chunks).sum();
                }
                Err(e) => self.error = Some(format!("{e:#}")),
            }
        }
        let link = &self.link;
        LinkStatus {
            name: link.name.clone(),
            kind: link.kind,
            root: self.prefix.trim_end_matches('/').to_owned(),
            path: link.root.to_string_lossy().into_owned(),
            git: link.config.git.clone(),
            branch: link.config.branch.clone(),
            description: link.config.description.clone(),
            memory: link.config.memory,
            index: link.index(),
            exists: link.exists(),
            files,
            chunks,
            error: self.error.clone(),
        }
    }
}

/// Prefix of a linked workspace's paths in this workspace's results.
pub(crate) fn prefix(link: &Link, workspace: &Workspace) -> String {
    let root = link.display_root(workspace.root());
    if root == "." { String::new() } else { format!("{}/", root.trim_end_matches('/')) }
}

/// Latest save of any partition under `databases` (`None` when nothing
/// was ever indexed there).
fn stamp(databases: &Path) -> Option<u128> {
    let mut manifests = vec![databases.join("cold").join(MANIFEST_FILE)];
    if let Ok(entries) = std::fs::read_dir(databases.join("hot")) {
        manifests.extend(entries.filter_map(Result::ok).map(|e| e.path().join(MANIFEST_FILE)));
    }
    manifests
        .iter()
        .filter_map(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        .filter_map(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .max()
}
