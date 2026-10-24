//! Thin wrappers around the `git` executable.
//!
//! Shelling out keeps the binary small and behaves exactly like the user's
//! own git (credentials helpers, LFS, `safe.directory`, ...).

use std::path::Path;
use std::process::Command;

use crate::error::{CoreError, Result};

/// Runs `git -C <dir> <args>` and returns trimmed stdout.
pub fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| CoreError::Git { args: args.join(" "), message: format!("cannot run git: {e}") })?;
    if !output.status.success() {
        return Err(CoreError::Git {
            args: args.join(" "),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim_end().to_owned())
}

/// Whether `dir` is inside a git work tree.
pub fn is_repository(dir: &Path) -> bool {
    run(dir, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out == "true")
}

/// Workspace relative paths (with `/`) of every initialized submodule,
/// recursively. Returns an empty list outside git repositories.
pub fn submodules(root: &Path) -> Vec<String> {
    if !root.join(".gitmodules").is_file() {
        return Vec::new();
    }
    match run(root, &["submodule", "status", "--recursive"]) {
        Ok(out) => parse_submodule_status(&out),
        Err(e) => {
            tracing::warn!(error = %e, "cannot list git submodules; indexing them as plain directories");
            Vec::new()
        }
    }
}

/// Parses `git submodule status` output:
/// `<status><sha> <path>[ (<describe>)]` where status is one of ` -+U`.
fn parse_submodule_status(output: &str) -> Vec<String> {
    let mut paths: Vec<String> = output
        .lines()
        .filter(|line| !line.starts_with('-')) // not initialized: no files on disk
        .filter_map(|line| {
            let rest = line.get(1..)?.trim_start();
            let (_sha, rest) = rest.split_once(' ')?;
            let path = match rest.rfind(" (") {
                Some(idx) if rest.ends_with(')') => &rest[..idx],
                _ => rest,
            };
            Some(path.replace('\\', "/"))
        })
        .collect();
    paths.sort();
    paths
}

/// Directories never searched for nested repositories.
const SKIP_DIRS: &[&str] = &[".git", ".brokspace", "node_modules", "target", "dist", "build", "vendor"];

/// Independent git repositories nested in the workspace (directories below
/// `root` holding a `.git` directory or file), up to `max_depth` levels deep.
/// Covers "a folder of repositories" workspaces; submodules show up too and
/// are deduplicated by the caller. Hidden directories are skipped.
pub fn nested_repositories(root: &Path, max_depth: usize) -> Vec<String> {
    fn visit(root: &Path, dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !path.is_dir() || name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if path.join(".git").exists()
                && let Some(rel) = crate::fsutil::to_workspace_relative(root, &path)
            {
                out.push(rel);
            }
            if depth < max_depth {
                visit(root, &path, depth + 1, max_depth, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, 1, max_depth, &mut out);
    out.sort();
    out
}

/// Runs `git lfs track <pattern>` in `root`.
pub fn lfs_track(root: &Path, pattern: &str) -> Result<()> {
    run(root, &["lfs", "install", "--local"])?;
    run(root, &["lfs", "track", pattern]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for repo in ["api", "libs/ui", "libs/ui/vendor-x", "node_modules/pkg", ".hidden/repo", "a/b/c/d/e/deep"] {
            std::fs::create_dir_all(root.join(repo).join(".git")).unwrap();
        }
        std::fs::create_dir_all(root.join("web")).unwrap();
        std::fs::write(root.join("web/.git"), "gitdir: ../.git/modules/web").unwrap(); // submodule style
        assert_eq!(nested_repositories(root, 4), vec!["api", "libs/ui", "libs/ui/vendor-x", "web"]);
    }

    #[test]
    fn parses_status_lines() {
        let out = " 1a2b3c libs/core (v1.0.0)\n+4d5e6f libs/ui (heads/main)\n-7a8b9c libs/missing\n 0f0f0f deep/path with space\n";
        assert_eq!(parse_submodule_status(out), vec!["deep/path with space", "libs/core", "libs/ui"]);
    }
}
