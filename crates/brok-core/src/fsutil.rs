//! Small file system helpers shared by every crate.

use std::io::Write;
use std::path::{Component, Path};

use crate::error::{CoreError, Result};

/// Writes `bytes` to `path` atomically: the data goes to a sibling temporary
/// file that is renamed over the destination, so readers never observe a
/// half-written file and a crash never corrupts the previous version.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent, e))?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| CoreError::io(path, e))
}

/// Marker file in folders that brok generated for other tools (e.g. skills
/// exported into `.claude/skills/`). They are never indexed and only ever
/// replaced by brok.
pub const MANAGED_MARKER: &str = ".brokspace-managed";

/// Whether the workspace relative `path` lies in a folder brok generated.
pub fn in_managed_dir(root: &Path, path: &str) -> bool {
    let mut dir = root.to_path_buf();
    let parts: Vec<&str> = path.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        dir.push(part);
        if dir.join(MANAGED_MARKER).is_file() {
            return true;
        }
    }
    false
}

/// Moves a directory, tolerating transient locks.
///
/// On Windows, antivirus scanners briefly lock freshly written executables,
/// which makes `rename` fail with "access denied". The move is retried with
/// backoff and finally falls back to copy + delete.
pub fn move_dir(from: &Path, to: &Path) -> Result<()> {
    let mut delay = std::time::Duration::from_millis(50);
    for _ in 0..8 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_secs(1));
            }
            Err(e) => return Err(CoreError::io(to, e)),
        }
    }
    copy_dir(from, to)?;
    let _ = std::fs::remove_dir_all(from);
    Ok(())
}

/// Recursively copies a directory tree.
pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| CoreError::io(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| CoreError::io(from, e))? {
        let entry = entry.map_err(|e| CoreError::io(from, e))?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| CoreError::io(&target, e))?;
        }
    }
    Ok(())
}

/// Converts `path` (absolute or relative to `root`) into a workspace
/// relative string with `/` separators. Returns `None` when the path lies
/// outside the workspace.
pub fn to_workspace_relative(root: &Path, path: &Path) -> Option<String> {
    let relative = if path.is_absolute() { path.strip_prefix(root).ok()? } else { path };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let root = std::env::temp_dir().join("ws");
        assert_eq!(to_workspace_relative(&root, &root.join("src").join("main.rs")).unwrap(), "src/main.rs");
        assert_eq!(to_workspace_relative(&root, Path::new("./a/../b.rs")).unwrap(), "b.rs");
        assert!(to_workspace_relative(&root, Path::new("../escape")).is_none());
        assert!(to_workspace_relative(&root, &std::env::temp_dir().join("other")).is_none());
    }

    #[test]
    fn atomic_write_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.txt");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }
}
