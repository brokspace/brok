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

/// Runs `git lfs track <pattern>` in `root`.
pub fn lfs_track(root: &Path, pattern: &str) -> Result<()> {
    run(root, &["lfs", "install", "--local"])?;
    run(root, &["lfs", "track", pattern]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_lines() {
        let out = " 1a2b3c libs/core (v1.0.0)\n+4d5e6f libs/ui (heads/main)\n-7a8b9c libs/missing\n 0f0f0f deep/path with space\n";
        assert_eq!(parse_submodule_status(out), vec!["deep/path with space", "libs/core", "libs/ui"]);
    }
}
