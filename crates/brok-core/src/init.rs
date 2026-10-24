//! `brok init`: scaffolds the `.brokspace/` directory.

use std::path::{Path, PathBuf};

use crate::config::DEFAULT_TEMPLATE;
use crate::error::{CoreError, Result};
use crate::fsutil::write_atomic;
use crate::workspace::{Workspace, WorkspacePaths, dunce_canonicalize};

/// Content of `.brokspace/.gitignore`.
pub const GITIGNORE: &str = "\
# Machine-local runtime state, managed by brok.
cache/
run/
logs/
shards/.staging/
shards/*/bin/
# The index is local by default. Remove the next line, or run `brok setup`
# and pick Git LFS, to version it with the repository.
databases/
";

/// Options for [`init_workspace`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InitOptions {
    /// Overwrite an existing `brok.yml` with the default template.
    pub force: bool,
}

/// What [`init_workspace`] did.
#[derive(Debug, Clone)]
pub struct InitReport {
    /// The opened workspace.
    pub workspace: Workspace,
    /// Files and directories created.
    pub created: Vec<PathBuf>,
    /// Whether an existing configuration was kept.
    pub kept_config: bool,
}

/// Creates the `.brokspace/` layout in `root`.
///
/// Idempotent: running it on an initialized workspace only fills in missing
/// directories, unless `force` is set, in which case `brok.yml` is reset.
pub fn init_workspace(root: &Path, options: InitOptions) -> Result<InitReport> {
    std::fs::create_dir_all(root).map_err(|e| CoreError::io(root, e))?;
    let paths = WorkspacePaths::new(dunce_canonicalize(root)?);
    let mut created = Vec::new();

    for dir in paths.scaffold_dirs() {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| CoreError::io(dir, e))?;
            created.push(dir.to_path_buf());
        }
    }

    let config_exists = paths.config_file.is_file();
    if !config_exists || options.force {
        write_atomic(&paths.config_file, DEFAULT_TEMPLATE.as_bytes())?;
        created.push(paths.config_file.clone());
    }

    let gitignore = paths.brokspace.join(".gitignore");
    if !gitignore.exists() {
        write_atomic(&gitignore, GITIGNORE.as_bytes())?;
        created.push(gitignore);
    }

    Ok(InitReport { workspace: Workspace::open(&paths.root)?, created, kept_config: config_exists && !options.force })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffolds_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let first = init_workspace(dir.path(), InitOptions::default()).unwrap();
        assert!(!first.kept_config);
        let paths = first.workspace.paths().clone();
        for d in paths.scaffold_dirs() {
            assert!(d.is_dir(), "{} missing", d.display());
        }

        std::fs::write(&paths.config_file, "version: 1\nsearch:\n  limit: 3\n").unwrap();
        let second = init_workspace(dir.path(), InitOptions::default()).unwrap();
        assert!(second.kept_config);
        assert_eq!(second.workspace.config().search.limit, 3);

        let forced = init_workspace(dir.path(), InitOptions { force: true }).unwrap();
        assert_eq!(forced.workspace.config().search.limit, 8);
    }
}
