//! Memory search: BM25 over the memory files of a workspace (private,
//! shared, global) and of its linked workspaces, plus one entry per linked
//! workspace that has a description.
//!
//! Memories are few and small, so they are not stored in the chunk store:
//! the folders are read when they change (their modification times and
//! sizes are compared on every search) and indexed in memory.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use brok_core::memory::{Kind, Memory, Scope, load_dir};
use brok_core::{Link, MemoryDirs, Workspace};
use brok_sdk::payload::SearchHit;

use crate::bm25::Bm25Index;

/// Characters of a memory returned in a search hit.
const MAX_HIT_CHARS: usize = 4_000;

/// One folder of memories.
#[derive(Debug, Clone)]
struct Folder {
    dir: PathBuf,
    scope: Scope,
    /// Name of the linked workspace, `None` for this workspace.
    origin: Option<String>,
    /// How paths in this folder are shown (prefix of the file name).
    shown_as: String,
}

/// A searchable memory.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) memory: Memory,
    pub(crate) origin: Option<String>,
    pub(crate) path: String,
}

struct Loaded {
    stamp: Vec<(u64, u64)>,
    entries: Vec<Entry>,
    bm25: Bm25Index,
}

/// The memories visible from a workspace.
pub(crate) struct MemorySearch {
    folders: Vec<Folder>,
    links: Vec<Entry>,
    loaded: Mutex<Option<Loaded>>,
}

impl MemorySearch {
    /// The memories of `workspace` and of the linked workspaces in `links`
    /// (with the path prefix their files are shown with).
    pub(crate) fn new(workspace: &Workspace, links: &[(Link, String)]) -> Self {
        let root = workspace.root();
        let mut folders = Vec::new();
        let dirs = MemoryDirs::of(workspace.paths(), workspace.config());
        for (scope, dir) in dirs.all() {
            let shown_as = brok_core::fsutil::to_workspace_relative(root, dir)
                .unwrap_or_else(|| dir.to_string_lossy().replace('\\', "/"));
            folders.push(Folder { dir: dir.to_path_buf(), scope, origin: None, shown_as });
        }

        let mut link_entries = Vec::new();
        for (link, prefix) in links {
            if link.config.memory {
                let mut add = |dir: PathBuf, scope: Scope, rel: &str| {
                    folders.push(Folder {
                        dir,
                        scope,
                        origin: Some(link.name.clone()),
                        shown_as: format!("{prefix}{rel}"),
                    });
                };
                add(link.root.join(brok_core::workspace::SHARED_MEMORY_DIR), Scope::Shared, ".brokspace-memory");
                if link.index() == brok_core::links::LinkIndex::Own {
                    add(link.root.join(".brokspace").join("memory"), Scope::Private, ".brokspace/memory");
                }
            }
            if let Some(description) = link.config.description.as_deref().filter(|d| !d.trim().is_empty()) {
                let location = match &link.config.git {
                    Some(url) => format!("git {url}, cloned in {}", prefix.trim_end_matches('/')),
                    None => prefix.trim_end_matches('/').to_owned(),
                };
                let memory = Memory {
                    id: format!("link-{}", link.name),
                    title: format!("Linked workspace `{}`", link.name),
                    kind: Kind::Link,
                    scope: Scope::Shared,
                    tags: vec!["link".into(), link.name.clone()],
                    created: String::new(),
                    updated: String::new(),
                    source: Some("brok.yml".into()),
                    body: format!(
                        "{description}\n\nThis workspace depends on `{}` ({location}); search results from it are \
                         prefixed with `{prefix}`.",
                        link.name
                    ),
                    path: workspace.paths().config_file.clone(),
                };
                let path = brok_core::fsutil::to_workspace_relative(root, &workspace.paths().config_file)
                    .unwrap_or_else(|| ".brokspace/brok.yml".into());
                link_entries.push(Entry { memory, origin: Some(link.name.clone()), path });
            }
        }
        Self { folders, links: link_entries, loaded: Mutex::new(None) }
    }

    /// The best memories for `query`, scored like a single ranked list of
    /// a partition (`lists` is the number of lists code partitions fuse, so
    /// a top memory is worth what a chunk found by one of them is).
    pub(crate) fn search(&self, query: &str, limit: usize, lists: usize) -> Vec<SearchHit> {
        let mut guard = self.loaded.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let stamp = self.stamp();
        if guard.as_ref().is_none_or(|loaded| loaded.stamp != stamp) {
            *guard = Some(self.load(stamp));
        }
        let Some(loaded) = guard.as_ref() else { return Vec::new() };
        let best = lists.max(1) as f32 / (crate::search::RRF_K + 1.0);
        loaded
            .bm25
            .search(query, limit)
            .into_iter()
            .enumerate()
            .filter_map(|(rank, (id, _))| {
                let entry = &loaded.entries[id.parse::<usize>().ok()?];
                let score = (1.0 / (crate::search::RRF_K + rank as f32 + 1.0)) / best;
                Some(hit(entry, score))
            })
            .collect()
    }

    /// Modification time and entry count of every folder.
    fn stamp(&self) -> Vec<(u64, u64)> {
        self.folders
            .iter()
            .map(|folder| {
                let modified = std::fs::metadata(&folder.dir)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
                let count = std::fs::read_dir(&folder.dir).map_or(0, |entries| entries.count() as u64);
                (modified, count)
            })
            .collect()
    }

    fn load(&self, stamp: Vec<(u64, u64)>) -> Loaded {
        let mut entries: Vec<Entry> = Vec::new();
        for folder in &self.folders {
            for memory in load_dir(&folder.dir, folder.scope) {
                let file = memory.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let path = format!("{}/{file}", folder.shown_as.trim_end_matches('/'));
                entries.push(Entry { memory, origin: folder.origin.clone(), path });
            }
        }
        entries.extend(self.links.iter().cloned());
        entries.sort_by(|a, b| b.memory.updated.cmp(&a.memory.updated));
        let mut bm25 = Bm25Index::default();
        for (i, entry) in entries.iter().enumerate() {
            let memory = &entry.memory;
            let text =
                format!("{}\n{}\n{}\n{}", memory.title, memory.tags.join(" "), memory.kind.as_str(), memory.body);
            bm25.add(&i.to_string(), &entry.path, &text);
        }
        Loaded { stamp, entries, bm25 }
    }
}

/// A memory as a search hit (`kind: "memory"`, the title as symbol).
pub(crate) fn hit(entry: &Entry, score: f32) -> SearchHit {
    let memory = &entry.memory;
    let mut content = format!("# {}\n\n{}", memory.title, memory.body);
    if content.chars().count() > MAX_HIT_CHARS {
        content = content.chars().take(MAX_HIT_CHARS).collect::<String>() + "\n…";
    }
    let partition = match &entry.origin {
        Some(name) => format!("@{name}/memory"),
        None => "memory".into(),
    };
    let mut signature = format!("{} · {}", memory.kind.as_str(), memory.scope.as_str());
    if !memory.updated.is_empty() {
        signature.push_str(&format!(" · {}", memory.updated.get(..10).unwrap_or(&memory.updated)));
    }
    SearchHit {
        chunk_id: format!("memory:{}:{}:{}", entry.origin.as_deref().unwrap_or(""), memory.scope.as_str(), memory.id),
        partition,
        path: entry.path.clone(),
        language: "markdown".into(),
        kind: "memory".into(),
        symbol: Some(memory.title.clone()),
        signature: Some(signature),
        start_line: 1,
        end_line: u32::try_from(content.lines().count()).unwrap_or(u32::MAX).max(1),
        score,
        content,
    }
}

/// Searches the memories of `workspace` and of its linked workspaces,
/// without opening the index (`brok memory search`).
pub fn search_memories(workspace: &Workspace, query: &str, limit: usize) -> Vec<SearchHit> {
    let links: Vec<(Link, String)> =
        Link::all(workspace).into_iter().map(|link| (link.clone(), crate::links::prefix(&link, workspace))).collect();
    MemorySearch::new(workspace, &links).search(query, limit, 1)
}
