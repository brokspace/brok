//! Memories: what Brokspace remembers between sessions.
//!
//! A memory is a Markdown file with a YAML front matter (title, kind, tags,
//! dates, source), so it can be read, edited and reviewed like any other
//! note. The files are the source of truth; search indexes them on the fly.
//!
//! ```text
//! <root>/.brokspace/memory/<id>.md   private: this workspace, never committed
//! <root>/.brokspace-memory/<id>.md   shared: committed, the team gets it with git
//! <config dir>/memory/<id>.md        global: yours, in every workspace
//! ```
//!
//! Forgetting a private or global memory moves it to `.forgotten/` next to
//! it (nothing is lost by mistake); a shared one is deleted, git keeps its
//! history. Secrets (API keys, tokens, private keys, passwords) are masked
//! before anything is written.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::BrokConfig;
use crate::error::{CoreError, Result};
use crate::workspace::WorkspacePaths;

/// Folder, next to the memories, where forgotten ones are moved.
pub const FORGOTTEN_DIR: &str = ".forgotten";
/// Longest body kept, in bytes (a memory is a note, not a document).
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// Where a memory lives, which decides who sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// This workspace, on this machine.
    Private,
    /// This workspace, for everyone who clones it.
    Shared,
    /// Every workspace of this user.
    Global,
}

impl Scope {
    /// Lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
            Self::Global => "global",
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "private" => Ok(Self::Private),
            "shared" => Ok(Self::Shared),
            "global" => Ok(Self::Global),
            other => Err(format!("unknown scope `{other}` (private, shared or global)")),
        }
    }
}

/// What a memory records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Anything worth keeping.
    #[default]
    Note,
    /// A decision and why it was taken.
    Decision,
    /// A fact about the code, the system or the team.
    Fact,
    /// A summary of a working session (saved by the IDE).
    Session,
    /// Where work stopped and what comes next.
    Handoff,
    /// A linked workspace (from `workspace.links`, never a file).
    Link,
}

impl Kind {
    /// Lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Decision => "decision",
            Self::Fact => "fact",
            Self::Session => "session",
            Self::Handoff => "handoff",
            Self::Link => "link",
        }
    }
}

impl std::str::FromStr for Kind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "note" => Ok(Self::Note),
            "decision" => Ok(Self::Decision),
            "fact" => Ok(Self::Fact),
            "session" => Ok(Self::Session),
            "handoff" => Ok(Self::Handoff),
            other => Err(format!("unknown kind `{other}` (note, decision, fact, session or handoff)")),
        }
    }
}

/// One memory.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Memory {
    /// File name without `.md`; unique within its scope.
    pub id: String,
    /// One line summary.
    pub title: String,
    /// What it records.
    pub kind: Kind,
    /// Where it lives.
    pub scope: Scope,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Creation time (RFC 3339, UTC).
    pub created: String,
    /// Last change (RFC 3339, UTC).
    pub updated: String,
    /// Who wrote it (`cli`, `ide`, `mcp`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Markdown body.
    pub body: String,
    /// The file.
    pub path: PathBuf,
}

/// What to remember.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewMemory {
    /// Replaces the memory with this id in the same scope (keeping its
    /// creation time) instead of creating one.
    pub id: Option<String>,
    /// One line summary.
    pub title: String,
    /// Markdown body.
    pub body: String,
    /// What it records.
    pub kind: Kind,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Who writes it.
    pub source: Option<String>,
}

/// Front matter as stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Front {
    title: String,
    kind: Kind,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    created: String,
    updated: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

/// The memory folders of a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDirs {
    /// `.brokspace/memory/`
    pub private: PathBuf,
    /// `.brokspace-memory/`
    pub shared: PathBuf,
    /// `<config dir>/memory/`, when global memories are enabled.
    pub global: Option<PathBuf>,
}

impl MemoryDirs {
    /// The folders of a workspace, as configured.
    pub fn of(paths: &WorkspacePaths, config: &BrokConfig) -> Self {
        Self {
            private: paths.memory.clone(),
            shared: paths.shared_memory.clone(),
            global: config.memory.global.then(global_dir).flatten(),
        }
    }

    /// Folder of a scope.
    pub fn dir(&self, scope: Scope) -> Result<&Path> {
        match scope {
            Scope::Private => Ok(&self.private),
            Scope::Shared => Ok(&self.shared),
            Scope::Global => self.global.as_deref().ok_or_else(|| {
                CoreError::Config("global memories are off (memory.global) or the config directory is unknown".into())
            }),
        }
    }

    /// Every folder with its scope.
    pub fn all(&self) -> Vec<(Scope, &Path)> {
        let mut dirs = vec![(Scope::Private, self.private.as_path()), (Scope::Shared, self.shared.as_path())];
        if let Some(global) = &self.global {
            dirs.push((Scope::Global, global.as_path()));
        }
        dirs
    }

    /// Every memory, most recently updated first.
    pub fn list(&self) -> Vec<Memory> {
        let mut memories: Vec<Memory> = self.all().into_iter().flat_map(|(scope, dir)| load_dir(dir, scope)).collect();
        memories.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
        memories
    }

    /// The memory with `id`, in `scope` or, when none is given, the first
    /// scope that has it (private, shared, global).
    pub fn find(&self, id: &str, scope: Option<Scope>) -> Option<Memory> {
        if !is_id(id) {
            return None;
        }
        self.all()
            .into_iter()
            .filter(|(s, _)| scope.is_none_or(|wanted| wanted == *s))
            .find_map(|(s, dir)| read(&dir.join(format!("{id}.md")), s))
    }

    /// Saves a memory in `scope`: a new one, or a replacement when `new.id`
    /// names an existing one.
    pub fn save(&self, scope: Scope, new: NewMemory) -> Result<Memory> {
        let dir = self.dir(scope)?;
        let title = mask_secrets(new.title.trim());
        if title.is_empty() {
            return Err(CoreError::Config("a memory needs a title".into()));
        }
        let mut body = mask_secrets(new.body.trim());
        if body.len() > MAX_BODY_BYTES {
            let mut end = MAX_BODY_BYTES;
            while !body.is_char_boundary(end) {
                end -= 1;
            }
            body.truncate(end);
            body.push_str("\n\n…");
        }
        let now = now_rfc3339();
        let (id, created) = match new.id {
            Some(id) => {
                if !is_id(&id) {
                    return Err(CoreError::Config(format!(
                        "`{id}` is not a valid memory id (letters, digits, `-` and `_`)"
                    )));
                }
                let created = read(&dir.join(format!("{id}.md")), scope).map_or_else(|| now.clone(), |m| m.created);
                (id, created)
            }
            None => (unique_id(dir, &title, &now), now.clone()),
        };
        let mut tags: Vec<String> = new.tags.iter().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect();
        tags.sort();
        tags.dedup();
        let front = Front { title, kind: new.kind, tags, created, updated: now, source: new.source };
        let yaml = serde_yaml_ng::to_string(&front).map_err(|e| CoreError::Config(e.to_string()))?;
        let path = dir.join(format!("{id}.md"));
        crate::fsutil::write_atomic(&path, format!("---\n{yaml}---\n\n{body}\n").as_bytes())?;
        read(&path, scope).ok_or_else(|| CoreError::Config(format!("{}: cannot read back the memory", path.display())))
    }

    /// Forgets a memory: private and global ones move to `.forgotten/`,
    /// shared ones are deleted (git has their history).
    pub fn forget(&self, id: &str, scope: Option<Scope>) -> Result<Memory> {
        let memory = self.find(id, scope).ok_or_else(|| CoreError::Config(format!("no memory `{id}`")))?;
        if memory.scope == Scope::Shared {
            std::fs::remove_file(&memory.path).map_err(|e| CoreError::io(&memory.path, e))?;
        } else {
            let dir = memory.path.parent().unwrap_or_else(|| Path::new(".")).join(FORGOTTEN_DIR);
            std::fs::create_dir_all(&dir).map_err(|e| CoreError::io(&dir, e))?;
            let target = dir.join(format!("{}.md", memory.id));
            std::fs::rename(&memory.path, &target).map_err(|e| CoreError::io(&target, e))?;
        }
        Ok(memory)
    }
}

/// The user's global memories (`<config dir>/memory`).
pub fn global_dir() -> Option<PathBuf> {
    crate::user::config_dir().map(|dir| dir.join("memory"))
}

/// Every memory file of `dir` (not recursive: `.forgotten/` is skipped).
pub fn load_dir(dir: &Path, scope: Scope) -> Vec<Memory> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut memories: Vec<Memory> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md") && path.is_file())
        .filter_map(|path| read(&path, scope))
        .collect();
    memories.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
    memories
}

/// Reads one memory file. Files without front matter are notes titled by
/// their first heading (or file name), so hand-written notes work too.
pub fn read(path: &Path, scope: Scope) -> Option<Memory> {
    let id = path.file_stem()?.to_str()?.to_owned();
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).replace("\r\n", "\n");
    let (front, body) = match split_front_matter(&text) {
        Some((yaml, body)) => (serde_yaml_ng::from_str::<Front>(yaml).unwrap_or_default(), body),
        None => (Front::default(), text.as_str()),
    };
    let body = body.trim().to_owned();
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok().map(rfc3339_at).unwrap_or_default();
    let title = if front.title.trim().is_empty() {
        body.lines()
            .find_map(|line| line.strip_prefix('#').map(|h| h.trim_start_matches('#').trim().to_owned()))
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| id.replace(['-', '_'], " "))
    } else {
        front.title
    };
    let created = if front.created.is_empty() { modified.clone() } else { front.created };
    let updated = if front.updated.is_empty() { created.clone() } else { front.updated };
    Some(Memory {
        id,
        title,
        kind: front.kind,
        scope,
        tags: front.tags,
        created,
        updated,
        source: front.source,
        body,
        path: path.to_path_buf(),
    })
}

fn split_front_matter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    if let Some(body) = rest.strip_prefix("---\n") {
        return Some(("", body));
    }
    let end = rest
        .find("\n---\n")
        .map(|i| (i, i + 5))
        .or_else(|| rest.strip_suffix("\n---").map(|r| (r.len(), rest.len())))?;
    Some((&rest[..end.0], &rest[end.1..]))
}

/// Whether `id` can name a memory file.
pub fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 120 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// `<date>-<slug of the title>`, numbered when taken.
fn unique_id(dir: &Path, title: &str, now: &str) -> String {
    let date = now.get(..10).unwrap_or("memory").replace('-', "");
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let base = format!("{date}-{}", slug.trim_end_matches('-'));
    let base = base.trim_end_matches('-').to_owned();
    let mut id = base.clone();
    let mut n = 2;
    while dir.join(format!("{id}.md")).exists() || dir.join(FORGOTTEN_DIR).join(format!("{id}.md")).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // Private keys.
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        // Provider tokens.
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}\b",
        r"\bgithub_pat_[A-Za-z0-9_]{40,}\b",
        r"\bglpat-[A-Za-z0-9_-]{20,}\b",
        r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b",
        r"\bsk-(?:ant-|proj-)?[A-Za-z0-9_-]{20,}\b",
        r"\b(?:sk|rk|pk)_(?:live|test)_[A-Za-z0-9]{16,}\b",
        r"\bAKIA[0-9A-Z]{16}\b",
        r"\bAIza[0-9A-Za-z_-]{35}\b",
        r"\bnpm_[A-Za-z0-9]{36}\b",
        // JSON web tokens.
        r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
        // Credentials in URLs.
        r"(?i)\b[a-z][a-z0-9+.-]*://[^\s:/@]+:[^\s@/]+@",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid secret pattern"))
    .collect()
});

static ASSIGNED_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([a-z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key)[a-z0-9_.-]*)(\s*[:=]\s*)(["']?)([^\s"',;]{6,})"#,
    )
    .expect("valid assignment pattern")
});

/// Replaces anything that looks like a secret with `[secret]`.
pub fn mask_secrets(text: &str) -> String {
    let mut out = text.to_owned();
    for pattern in SECRETS.iter() {
        out = pattern.replace_all(&out, "[secret]").into_owned();
    }
    ASSIGNED_SECRET.replace_all(&out, "$1$2$3[secret]").into_owned()
}

/// The current time, RFC 3339 in UTC with seconds.
pub fn now_rfc3339() -> String {
    rfc3339_at(SystemTime::now())
}

/// `time` as RFC 3339 in UTC with seconds.
pub fn rfc3339_at(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Days to civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> MemoryDirs {
        MemoryDirs { private: root.join("private"), shared: root.join("shared"), global: Some(root.join("global")) }
    }

    fn note(title: &str, body: &str) -> NewMemory {
        NewMemory { title: title.into(), body: body.into(), ..NewMemory::default() }
    }

    #[test]
    fn save_list_find_and_forget() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(tmp.path());
        let decision = NewMemory {
            kind: Kind::Decision,
            tags: vec!["deploy".into(), "deploy".into(), " ".into()],
            ..note("Use pnpm 10 on Hostinger", "Corepack cannot start pnpm 12.")
        };
        let saved = dirs.save(Scope::Private, decision).unwrap();
        assert!(saved.id.ends_with("-use-pnpm-10-on-hostinger"), "{}", saved.id);
        assert_eq!(saved.tags, vec!["deploy"]);
        assert_eq!(saved.kind, Kind::Decision);
        dirs.save(Scope::Shared, note("Orders API", "checkout exposes /orders")).unwrap();
        dirs.save(Scope::Global, note("Prefer pnpm", "")).unwrap();

        assert_eq!(dirs.list().len(), 3);
        assert_eq!(dirs.find(&saved.id, None).unwrap().body, "Corepack cannot start pnpm 12.");
        assert!(dirs.find(&saved.id, Some(Scope::Shared)).is_none());

        // Same title again: a new id.
        let again = dirs.save(Scope::Private, note("Use pnpm 10 on Hostinger", "x")).unwrap();
        assert_ne!(again.id, saved.id);

        dirs.forget(&saved.id, None).unwrap();
        assert!(dirs.find(&saved.id, None).is_none());
        assert!(tmp.path().join("private").join(FORGOTTEN_DIR).join(format!("{}.md", saved.id)).is_file());
    }

    #[test]
    fn upsert_keeps_created() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = dirs(tmp.path());
        let first = dirs.save(Scope::Private, NewMemory { id: Some("session-1".into()), ..note("Q1", "a") }).unwrap();
        let second =
            dirs.save(Scope::Private, NewMemory { id: Some("session-1".into()), ..note("Q1", "a b") }).unwrap();
        assert_eq!(first.created, second.created);
        assert_eq!(second.body, "a b");
        assert_eq!(dirs.list().len(), 1);
        assert!(dirs.save(Scope::Private, NewMemory { id: Some("../x".into()), ..note("t", "") }).is_err());
    }

    #[test]
    fn hand_written_notes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("shared")).unwrap();
        std::fs::write(tmp.path().join("shared").join("payments.md"), "# Payments flow\r\n\r\nStripe webhooks.\r\n")
            .unwrap();
        let memory = dirs(tmp.path()).find("payments", None).unwrap();
        assert_eq!(memory.title, "Payments flow");
        assert_eq!(memory.scope, Scope::Shared);
        assert_eq!(memory.kind, Kind::Note);
        assert!(!memory.updated.is_empty());
    }

    #[test]
    fn secrets_are_masked() {
        let text = "token ghp_abcdefghijklmnopqrstuvwxyz0123456789 and DB_PASSWORD=hunter2hunter \
                    url https://bob:s3cret@example.com key AKIAABCDEFGHIJKLMNOP";
        let masked = mask_secrets(text);
        assert!(!masked.contains("ghp_abc"), "{masked}");
        assert!(!masked.contains("hunter2"), "{masked}");
        assert!(!masked.contains("s3cret"), "{masked}");
        assert!(!masked.contains("AKIAABCD"), "{masked}");
        assert!(masked.contains("DB_PASSWORD=[secret]"), "{masked}");
        assert_eq!(mask_secrets("the token expires after 10 minutes"), "the token expires after 10 minutes");
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339_at(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_at(UNIX_EPOCH + std::time::Duration::from_secs(1_793_619_890)), "2026-11-02T11:44:50Z");
    }
}
