//! A whole Obsidian vault as searchable notes, read only.
//!
//! Hidden folders (`.obsidian/`, `.trash/`) and the configured exclusions
//! are skipped, symbolic links are not followed, and both one note and the
//! whole vault have a size budget, so a huge vault costs a bounded amount of
//! memory instead of growing with it.

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use brok_core::Memory;
use brok_core::memory::read_vault_note;
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

/// The most notes read from a vault.
const MAX_NOTES: usize = 20_000;
/// The most bytes of note text read from a vault.
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Characters of a note kept: enough to search and to answer from.
const MAX_NOTE_CHARS: usize = 8_000;

/// A vault to search.
pub(crate) struct Vault {
    root: PathBuf,
    exclude: Vec<String>,
    max_file_bytes: u64,
}

impl Vault {
    pub(crate) fn new(root: PathBuf, exclude: Vec<String>, max_file_bytes: u64) -> Self {
        Self { root, exclude, max_file_bytes }
    }

    /// The vault's path as results show it.
    pub(crate) fn shown_as(&self) -> String {
        self.root.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_owned()
    }

    /// Every note file within the rules: `(path, path in the vault, size, modified)`,
    /// in path order so the budget always cuts the same notes.
    fn files(&self) -> Vec<(PathBuf, String, u64, u64)> {
        let mut overrides = OverrideBuilder::new(&self.root);
        for pattern in &self.exclude {
            let pattern = pattern.trim().trim_matches(['/', '\\']).replace('\\', "/");
            // `!` excludes in an override; a folder also excludes what is under it.
            for glob in [format!("!{pattern}"), format!("!{pattern}/**")] {
                if let Err(error) = overrides.add(&glob) {
                    tracing::warn!(%error, pattern, "ignoring an invalid memory.obsidian.exclude entry");
                }
            }
        }
        let mut walker = WalkBuilder::new(&self.root);
        walker.standard_filters(false).hidden(true).follow_links(false);
        if let Ok(overrides) = overrides.build() {
            walker.overrides(overrides);
        }
        let mut files: Vec<_> = walker
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
            .filter(|entry| entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                if meta.len() > self.max_file_bytes {
                    return None;
                }
                let relative = entry.path().strip_prefix(&self.root).ok()?.to_str()?.replace('\\', "/");
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
                Some((entry.into_path(), relative, meta.len(), modified))
            })
            .collect();
        files.sort_by(|a, b| a.1.cmp(&b.1));
        files
    }

    /// Changes whenever a note is added, removed or edited.
    pub(crate) fn stamp(&self) -> (u64, u64) {
        self.files().iter().fold((0u64, 0u64), |(newest, mix), (_, relative, size, modified)| {
            let name = relative.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
            (newest.max(*modified), mix.wrapping_add(name ^ size.wrapping_mul(0x9E37_79B9) ^ modified))
        })
    }

    /// The notes with their path in the vault.
    pub(crate) fn load(&self) -> Vec<(Memory, String)> {
        let files = self.files();
        let total = files.len();
        let mut bytes = 0u64;
        let notes: Vec<_> = files
            .into_iter()
            .take(MAX_NOTES)
            .take_while(|(_, _, size, _)| {
                bytes = bytes.saturating_add(*size);
                bytes <= MAX_TOTAL_BYTES
            })
            .filter_map(|(path, relative, _, _)| {
                let mut memory = read_vault_note(&path, &relative)?;
                if let Some((cut, _)) = memory.body.char_indices().nth(MAX_NOTE_CHARS) {
                    memory.body.truncate(cut);
                    memory.body.shrink_to_fit();
                }
                Some((memory, relative))
            })
            .collect();
        if notes.len() < total {
            tracing::warn!(
                searched = notes.len(),
                found = total,
                "the vault is larger than the search budget; exclude folders with memory.obsidian.exclude"
            );
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &std::path::Path, path: &str, text: &str) {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn vault(root: &std::path::Path, exclude: &[&str]) -> Vault {
        Vault::new(root.into(), exclude.iter().map(|e| (*e).to_owned()).collect(), 1024)
    }

    #[test]
    fn reads_every_note_but_hidden_excluded_large_and_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "Ideias de verão.md", "a");
        write(root, "Projetos/My Plan.md", "b");
        write(root, "Private/secret.md", "c");
        write(root, "Brokspace/memory.md", "d");
        write(root, ".obsidian/plugins/readme.md", "e");
        write(root, ".trash/old.md", "f");
        write(root, "image.png", "g");
        write(root, "huge.md", &"x".repeat(4096));
        let found: Vec<_> = vault(root, &["Private", "Brokspace"]).load().into_iter().map(|(_, path)| path).collect();
        assert_eq!(found, ["Ideias de verão.md", "Projetos/My Plan.md"]);
    }

    #[test]
    fn the_stamp_follows_edits_additions_and_removals_anywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "a/b/c.md", "one");
        let vault = vault(root, &[]);
        let first = vault.stamp();
        assert_eq!(vault.stamp(), first);
        write(root, "a/b/c.md", "one two");
        let edited = vault.stamp();
        assert_ne!(edited, first);
        write(root, "a/b/d.md", "new");
        let added = vault.stamp();
        assert_ne!(added, edited);
        std::fs::remove_file(root.join("a/b/d.md")).unwrap();
        assert_ne!(vault.stamp(), added);
    }
}
