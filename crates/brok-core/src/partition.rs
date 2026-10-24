//! Hierarchical index partitions.
//!
//! The workspace is split along git submodule boundaries:
//!
//! - the root repository forms the **cold** partition, a single federated
//!   database under `.brokspace/databases/cold/`;
//! - every (nested) submodule forms a **hot** partition with its own
//!   micro-database under `.brokspace/databases/hot/<id>/`, so edits inside a
//!   submodule only ever rewrite that small database.
//!
//! Queries search the partition the user is working in first and fall back
//! to the others (see `brok-index`'s federated search).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace::WorkspacePaths;

/// Storage tier of a partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Root workspace, federated cold storage.
    Cold,
    /// Git submodule, incremental hot storage.
    Hot,
}

/// One indexable slice of the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Partition {
    /// Stable identifier (`root` or a slug of the submodule path).
    pub id: String,
    /// Workspace relative path with `/` separators (`""` for the root).
    pub path: String,
    /// Storage tier.
    pub tier: Tier,
}

impl Partition {
    /// Database directory of this partition.
    pub fn database_dir(&self, paths: &WorkspacePaths) -> PathBuf {
        match self.tier {
            Tier::Cold => paths.databases.join("cold"),
            Tier::Hot => paths.databases.join("hot").join(&self.id),
        }
    }

    /// Absolute directory of the partition's sources.
    pub fn source_dir(&self, root: &Path) -> PathBuf {
        if self.path.is_empty() { root.to_path_buf() } else { root.join(&self.path) }
    }

    /// Whether the workspace relative `path` belongs to this partition's
    /// subtree (ignoring nested partitions).
    pub fn contains(&self, path: &str) -> bool {
        self.path.is_empty()
            || path == self.path
            || path.strip_prefix(&self.path).is_some_and(|rest| rest.starts_with('/'))
    }
}

/// All partitions of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionMap {
    /// Sorted by descending path length so the first match is the deepest.
    partitions: Vec<Partition>,
}

impl PartitionMap {
    /// Builds the map from submodule paths (relative, `/` separated).
    pub fn new(submodules: impl IntoIterator<Item = String>) -> Self {
        let mut partitions = vec![Partition { id: "root".into(), path: String::new(), tier: Tier::Cold }];
        for path in submodules {
            let path = path.trim_matches('/').to_owned();
            if path.is_empty() {
                continue;
            }
            let mut id = slug(&path);
            while partitions.iter().any(|p| p.id == id) {
                id.push('_');
            }
            partitions.push(Partition { id, path, tier: Tier::Hot });
        }
        partitions.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then_with(|| a.path.cmp(&b.path)));
        Self { partitions }
    }

    /// Discovers submodules of the workspace (when enabled).
    pub fn discover(root: &Path, include_submodules: bool) -> Self {
        if include_submodules { Self::new(crate::git::submodules(root)) } else { Self::new([]) }
    }

    /// The deepest partition containing `path`.
    pub fn resolve(&self, path: &str) -> &Partition {
        self.partitions.iter().find(|p| p.contains(path)).unwrap_or_else(|| self.root())
    }

    /// The root (cold) partition.
    pub fn root(&self) -> &Partition {
        self.partitions.iter().find(|p| p.tier == Tier::Cold).expect("root partition always exists")
    }

    /// Looks a partition up by id.
    pub fn get(&self, id: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.id == id)
    }

    /// Partitions nested directly or indirectly inside `parent`.
    pub fn nested_in<'a>(&'a self, parent: &'a Partition) -> impl Iterator<Item = &'a Partition> + 'a {
        self.partitions.iter().filter(move |p| p.id != parent.id && parent.contains(&p.path))
    }

    /// Iterates partitions, hot ones first.
    pub fn iter(&self) -> impl Iterator<Item = &Partition> {
        self.partitions.iter()
    }

    /// Number of partitions.
    pub fn len(&self) -> usize {
        self.partitions.len()
    }

    /// Always false: the root partition exists.
    pub fn is_empty(&self) -> bool {
        false
    }
}

fn slug(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_deepest_partition() {
        let map = PartitionMap::new(["libs/core".to_string(), "libs/core/vendor/x".to_string()]);
        assert_eq!(map.resolve("libs/core/vendor/x/a.rs").id, "libs-core-vendor-x");
        assert_eq!(map.resolve("libs/core/src/lib.rs").id, "libs-core");
        assert_eq!(map.resolve("libs/corex/a.rs").id, "root");
        assert_eq!(map.resolve("README.md").tier, Tier::Cold);
    }

    #[test]
    fn nested_partitions() {
        let map = PartitionMap::new(["a".to_string(), "a/b".to_string(), "c".to_string()]);
        let root_nested: Vec<_> = map.nested_in(map.root()).map(|p| p.path.as_str()).collect();
        assert_eq!(root_nested.len(), 3);
        let a = map.get("a").unwrap();
        assert_eq!(map.nested_in(a).map(|p| p.path.as_str()).collect::<Vec<_>>(), vec!["a/b"]);
    }

    #[test]
    fn database_dirs() {
        let paths = WorkspacePaths::new("/ws");
        let map = PartitionMap::new(["Libs/UI Kit".to_string()]);
        let hot = map.get("libs-ui-kit").unwrap();
        assert!(hot.database_dir(&paths).ends_with(Path::new("hot").join("libs-ui-kit")));
        assert!(map.root().database_dir(&paths).ends_with("cold"));
    }
}
