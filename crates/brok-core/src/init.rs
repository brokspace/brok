//! `brok init`: scaffolds the `.brokspace/` directory.

use std::path::{Path, PathBuf};

use crate::config::DEFAULT_TEMPLATE;
use crate::error::{CoreError, Result};
use crate::fsutil::write_atomic;
use crate::workspace::{BROKSPACE_DIR, Workspace, WorkspacePaths, dunce_canonicalize};

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
# Personal or private to this machine: never commit these. To share one on
# purpose, add a rule such as `!/memory/`.
/memory/
/metrics/
/members/
/links/
/shards-data/
/reports/
";

/// Folders of `.brokspace/` holding personal or private data.
pub const PRIVATE_DIRS: [&str; 6] = ["memory", "metrics", "members", "links", "shards-data", "reports"];

const PRIVATE_HEADER: &str = "\
# Personal or private to this machine: never commit these (added by brok). To
# share one on purpose, add a rule such as `!/memory/`.
";

/// Makes sure `<state>/.gitignore` keeps the private folders out of git, for
/// workspaces created before it did. Lines the user wrote are kept, and a rule
/// that mentions a folder, ignoring it or opting it in with `!`, leaves that
/// folder alone. Returns the rules added. Does nothing unless `state` is an
/// existing `.brokspace` directory.
pub fn protect_private_files(state: &Path) -> Result<Vec<String>> {
    if state.file_name().is_none_or(|name| name != BROKSPACE_DIR) || !state.is_dir() {
        return Ok(Vec::new());
    }
    let path = state.join(".gitignore");
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // Without the file even the session token could be committed.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write_atomic(&path, GITIGNORE.as_bytes())?;
            return Ok(PRIVATE_DIRS.map(|dir| format!("/{dir}/")).to_vec());
        }
        Err(e) => return Err(CoreError::io(&path, e)),
    };
    let mentioned = |dir: &str| {
        current.lines().map(str::trim).any(|line| {
            if line.starts_with('#') {
                return false;
            }
            let rule = line.trim_start_matches('!').trim_start_matches("**/").trim_start_matches('/');
            let rule = rule.trim_end_matches("**").trim_end_matches('*').trim_end_matches('/');
            line == "*"
                || rule == dir
                || rule.strip_prefix(dir).is_some_and(|rest| rest.starts_with('/') && line.starts_with('!'))
        })
    };
    let missing: Vec<String> =
        PRIVATE_DIRS.iter().filter(|dir| !mentioned(dir)).map(|dir| format!("/{dir}/")).collect();
    if missing.is_empty() {
        return Ok(missing);
    }
    let separator = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
    let updated = format!("{current}{separator}{PRIVATE_HEADER}{}\n", missing.join("\n"));
    write_atomic(&path, updated.as_bytes())?;
    Ok(missing)
}

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
    let existed = gitignore.exists();
    protect_private_files(&paths.brokspace)?;
    if !existed {
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

    /// What `git add .` would commit under `.brokspace/`, with real git.
    fn would_commit(root: &Path) -> Vec<String> {
        let listed =
            crate::git::run(root, &["status", "--porcelain", "--untracked-files=all", "--", ".brokspace"]).unwrap();
        listed.lines().map(|line| line[3..].to_owned()).collect()
    }

    fn private_files(root: &Path) {
        for dir in PRIVATE_DIRS {
            let nested = root.join(".brokspace").join(dir).join("nested");
            std::fs::create_dir_all(&nested).unwrap();
            std::fs::write(nested.join("secret.md"), "private").unwrap();
        }
    }

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::git::run(dir.path(), &["init", "-q"]).unwrap();
        dir
    }

    #[test]
    fn a_new_workspace_commits_nothing_private() {
        let dir = repository();
        init_workspace(dir.path(), InitOptions::default()).unwrap();
        private_files(dir.path());
        // The rules are anchored: a skill that shares a name is still committed.
        let skill = dir.path().join(".brokspace/skills/reports");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "x").unwrap();
        assert_eq!(
            would_commit(dir.path()),
            [".brokspace/.gitignore", ".brokspace/brok.yml", ".brokspace/skills/reports/SKILL.md"]
        );
        assert!(protect_private_files(&dir.path().join(".brokspace")).unwrap().is_empty());
    }

    #[test]
    fn an_older_workspace_is_repaired_keeping_the_users_lines() {
        let dir = repository();
        init_workspace(dir.path(), InitOptions::default()).unwrap();
        let state = dir.path().join(".brokspace");
        // As brok 0.10 wrote it, edited by the user, without a final newline.
        std::fs::write(state.join(".gitignore"), "cache/\nrun/\n# memory/ stays\nmy-notes.txt").unwrap();
        private_files(dir.path());
        assert!(would_commit(dir.path()).iter().any(|f| f.contains("memory/")), "the problem");

        let added = protect_private_files(&state).unwrap();
        assert_eq!(added.len(), PRIVATE_DIRS.len());
        assert_eq!(would_commit(dir.path()), [".brokspace/.gitignore", ".brokspace/brok.yml"]);
        let text = std::fs::read_to_string(state.join(".gitignore")).unwrap();
        assert!(text.starts_with("cache/\nrun/\n# memory/ stays\nmy-notes.txt\n"), "{text}");
        assert!(protect_private_files(&state).unwrap().is_empty(), "once");
        assert_eq!(std::fs::read_to_string(state.join(".gitignore")).unwrap(), text);
    }

    #[test]
    fn rules_the_user_wrote_are_respected() {
        let dir = repository();
        init_workspace(dir.path(), InitOptions::default()).unwrap();
        let state = dir.path().join(".brokspace");
        std::fs::write(state.join(".gitignore"), "!memory/\n/metrics\n**/links/**\n!shards-data/team/\n").unwrap();
        assert_eq!(protect_private_files(&state).unwrap(), ["/members/", "/reports/"]);

        std::fs::write(state.join(".gitignore"), "*\n").unwrap();
        assert!(protect_private_files(&state).unwrap().is_empty(), "everything is ignored already");

        std::fs::remove_file(state.join(".gitignore")).unwrap();
        protect_private_files(&state).unwrap();
        assert_eq!(std::fs::read_to_string(state.join(".gitignore")).unwrap(), GITIGNORE);
    }

    #[test]
    fn other_directories_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        assert!(protect_private_files(dir.path()).unwrap().is_empty());
        assert!(protect_private_files(&dir.path().join(".brokspace")).unwrap().is_empty());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
