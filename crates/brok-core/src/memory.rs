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
//! <vault>/<folder>/<id>.md           obsidian: a folder of your Obsidian vault
//! ```
//!
//! Forgetting a private or global memory moves it to `.forgotten/` next to
//! it (nothing is lost by mistake); a shared one is deleted, git keeps its
//! history. Secrets (API keys, tokens, private keys, passwords) are masked
//! before anything is written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::config::{BrokConfig, ObsidianNames};
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
    /// A folder of the user's Obsidian vault (`memory.obsidian`).
    Obsidian,
}

impl Scope {
    /// Lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
            Self::Global => "global",
            Self::Obsidian => "obsidian",
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
            "obsidian" => Ok(Self::Obsidian),
            other => Err(format!("unknown scope `{other}` (private, shared, global or obsidian)")),
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
    /// A summary of a working session (saved by editor extensions).
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

/// Longest file name (without `.md`) made from a title.
const MAX_NAME_CHARS: usize = 80;
/// Titles shorter than this are too common to count as a mention.
const MIN_MENTION_CHARS: usize = 8;
/// Most related memories linked from one note.
const MAX_RELATED: usize = 5;
/// Most mentioned files listed in one note.
const MAX_FILES: usize = 10;

/// What `export_to_obsidian` did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Export {
    /// Notes created or updated.
    pub written: usize,
    /// Notes already up to date.
    pub unchanged: usize,
}

/// One memory.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Memory {
    /// Unique within its scope: the file name without `.md`, or the `id` property of an Obsidian note.
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
    /// Kept in Obsidian notes, whose file may be named after the title or renamed by the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// Other names of an Obsidian note, so `[[id]]` links reach it.
    #[serde(skip_serializing_if = "serde_yaml_ng::Value::is_null")]
    aliases: serde_yaml_ng::Value,
    /// Wikilinks to related memories (Obsidian draws them in the graph).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related: Vec<String>,
    /// Workspace files the note mentions.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<String>,
    /// Properties the user added: kept when brok rewrites the note.
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml_ng::Value>,
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
    /// `<vault>/<folder>/`, when an Obsidian vault is configured and exists.
    pub obsidian: Option<PathBuf>,
    /// How new notes in the vault are named (`memory.obsidian.names`).
    pub obsidian_names: ObsidianNames,
}

impl MemoryDirs {
    /// The folders of a workspace, as configured.
    pub fn of(paths: &WorkspacePaths, config: &BrokConfig) -> Self {
        Self {
            private: paths.memory.clone(),
            shared: paths.shared_memory.clone(),
            global: config.memory.global.then(global_dir).flatten(),
            obsidian: obsidian_dir(paths, config),
            obsidian_names: config.memory.obsidian.names,
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
            Scope::Obsidian => self.obsidian.as_deref().ok_or_else(|| {
                CoreError::Config(
                    "no Obsidian vault: set memory.obsidian.vault in brok.yml to an existing vault folder \
                     (is the vault's sync folder mounted?)"
                        .into(),
                )
            }),
        }
    }

    /// Every folder with its scope.
    pub fn all(&self) -> Vec<(Scope, &Path)> {
        let mut dirs = vec![(Scope::Private, self.private.as_path()), (Scope::Shared, self.shared.as_path())];
        if let Some(global) = &self.global {
            dirs.push((Scope::Global, global.as_path()));
        }
        if let Some(obsidian) = &self.obsidian {
            dirs.push((Scope::Obsidian, obsidian.as_path()));
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
            .find_map(|(s, dir)| find_in(dir, s, id))
    }

    /// Saves a memory in `scope`: a new one, or a replacement when `new.id`
    /// names an existing one.
    pub fn save(&self, scope: Scope, new: NewMemory) -> Result<Memory> {
        self.write(scope, new, None)
    }

    /// `save`, optionally keeping the creation and update times of a copied memory.
    fn write(&self, scope: Scope, new: NewMemory, stamps: Option<(String, String)>) -> Result<Memory> {
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
        if let Some(id) = new.id.as_deref().filter(|id| !is_id(id)) {
            return Err(CoreError::Config(format!("`{id}` is not a valid memory id (letters, digits, `-` and `_`)")));
        }
        // The note being replaced, wherever its file now is.
        let existing = new.id.as_deref().and_then(|id| find_in(dir, scope, id));
        let id = match new.id {
            Some(id) => id,
            None => unique_id(dir, scope, &title, &now),
        };
        let path = match &existing {
            Some(memory) => memory.path.clone(),
            None if scope == Scope::Obsidian && self.obsidian_names == ObsidianNames::Title => {
                dir.join(format!("{}.md", unique_name(dir, &title, &id)))
            }
            None => dir.join(format!("{id}.md")),
        };
        let (created, updated) = match stamps {
            Some(stamps) => stamps,
            None => (existing.as_ref().map_or_else(|| now.clone(), |m| m.created.clone()), now),
        };
        let mut tags: Vec<String> = new.tags.iter().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect();
        tags.sort();
        tags.dedup();
        let mut front = Front { title, kind: new.kind, tags, created, updated, source: new.source, ..Front::default() };
        if scope == Scope::Obsidian {
            if let Some(old) = existing.as_ref().and_then(|m| read_front(&m.path)) {
                front.aliases = old.aliases;
                front.extra = old.extra;
            }
            front.id = Some(id.clone());
            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&id);
            if file_stem != id && front.aliases.is_null() {
                front.aliases = serde_yaml_ng::Value::Sequence(vec![id.clone().into()]);
            }
            front.related = related_links(&load_dir(dir, scope), &id, &front.title, &body, &front.tags);
            front.files = mentioned_files(&body);
        }
        let yaml = serde_yaml_ng::to_string(&front).map_err(|e| CoreError::Config(e.to_string()))?;
        crate::fsutil::write_atomic(&path, format!("---\n{yaml}---\n\n{body}\n").as_bytes())?;
        read(&path, scope).ok_or_else(|| CoreError::Config(format!("{}: cannot read back the memory", path.display())))
    }

    /// Copies the private and shared memories into the Obsidian folder, keeping
    /// their ids and dates. A note already there and not older is left alone,
    /// so running it again changes nothing. Session records are not copied.
    pub fn export_to_obsidian(&self) -> Result<Export> {
        let target = self.dir(Scope::Obsidian)?;
        let mut export = Export::default();
        let mut sources: Vec<Memory> = [(Scope::Private, &self.private), (Scope::Shared, &self.shared)]
            .into_iter()
            .flat_map(|(scope, dir)| load_dir(dir, scope))
            .filter(|m| m.kind != Kind::Session && is_id(&m.id))
            .collect();
        // Oldest first, so later notes can link to earlier ones.
        sources.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id)));
        for memory in sources {
            if find_in(target, Scope::Obsidian, &memory.id).is_some_and(|there| there.updated >= memory.updated) {
                export.unchanged += 1;
                continue;
            }
            let new = NewMemory {
                id: Some(memory.id),
                title: memory.title,
                body: memory.body,
                kind: memory.kind,
                tags: memory.tags,
                source: memory.source,
            };
            self.write(Scope::Obsidian, new, Some((memory.created, memory.updated)))?;
            export.written += 1;
        }
        Ok(export)
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
            let name = memory.path.file_stem().and_then(|s| s.to_str()).unwrap_or(&memory.id);
            let mut target = dir.join(format!("{name}.md"));
            if target.exists() {
                target = dir.join(format!("{name} {}.md", memory.id));
            }
            std::fs::rename(&memory.path, &target).map_err(|e| CoreError::io(&target, e))?;
        }
        Ok(memory)
    }
}

/// The Obsidian folder memories go to, `<vault>/<folder>`.
///
/// `None` unless a vault is configured and **already exists**: brok does not
/// create a vault, so an unmounted cloud-sync folder never makes it write
/// memories to a lookalike path.
pub fn obsidian_dir(paths: &WorkspacePaths, config: &BrokConfig) -> Option<PathBuf> {
    let obsidian = &config.memory.obsidian;
    let vault = obsidian.vault.as_deref().map(str::trim).filter(|v| !v.is_empty())?;
    if !obsidian.folder_is_valid() {
        return None;
    }
    let vault = crate::links::absolute(&paths.root, vault);
    let folder: PathBuf = obsidian.folder.trim().split(['/', '\\']).collect();
    vault.is_dir().then(|| vault.join(folder))
}

/// The vault to search as a whole, when `memory.obsidian.search` is `vault`
/// and the vault exists.
pub fn obsidian_vault(paths: &WorkspacePaths, config: &BrokConfig) -> Option<PathBuf> {
    let obsidian = &config.memory.obsidian;
    if obsidian.search != crate::config::ObsidianSearch::Vault {
        return None;
    }
    let vault = obsidian.vault.as_deref().map(str::trim).filter(|v| !v.is_empty())?;
    let vault = crate::links::absolute(&paths.root, vault);
    vault.is_dir().then_some(vault)
}

/// A note anywhere in a vault, read only. Its id is its path in the vault
/// (spaces and unicode included), its tags come from the Obsidian properties
/// (a list or `a, b` text) and `aliases` join the tags so they are findable.
/// Secrets in the text are masked, because the result may reach an LLM.
pub fn read_vault_note(path: &Path, relative: &str) -> Option<Memory> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).replace("\r\n", "\n");
    let (front, body) = match split_front_matter(&text) {
        Some((yaml, body)) => (serde_yaml_ng::from_str::<serde_yaml_ng::Value>(yaml).unwrap_or_default(), body),
        None => (serde_yaml_ng::Value::Null, text.as_str()),
    };
    let id = relative.strip_suffix(".md").unwrap_or(relative).to_owned();
    let name = id.rsplit('/').next().unwrap_or(&id).to_owned();
    let title = front
        .get("title")
        .and_then(serde_yaml_ng::Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map_or(name, str::to_owned);
    let mut tags = property_list(&front, "tags");
    tags.extend(property_list(&front, "aliases"));
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok().map(rfc3339_at).unwrap_or_default();
    Some(Memory {
        id,
        title: mask_secrets(&title),
        kind: Kind::Note,
        scope: Scope::Obsidian,
        tags: tags.iter().map(|tag| mask_secrets(tag)).collect(),
        created: modified.clone(),
        updated: modified,
        source: None,
        body: mask_secrets(body.trim()),
        path: path.to_path_buf(),
    })
}

/// An Obsidian property that is a list, or text separated by commas.
fn property_list(front: &serde_yaml_ng::Value, key: &str) -> Vec<String> {
    use serde_yaml_ng::Value;
    let clean = |s: &str| s.trim().trim_start_matches('#').to_owned();
    let values: Vec<String> = match front.get(key) {
        Some(Value::Sequence(items)) => items.iter().filter_map(Value::as_str).map(clean).collect(),
        Some(Value::String(text)) => text.split(',').map(clean).collect(),
        _ => Vec::new(),
    };
    values.into_iter().filter(|v| !v.is_empty()).collect()
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
    let stem = path.file_stem()?.to_str()?.to_owned();
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).replace("\r\n", "\n");
    let (front, body) = match split_front_matter(&text) {
        Some((yaml, body)) => (serde_yaml_ng::from_str::<Front>(yaml).unwrap_or_default(), body),
        None => (Front::default(), text.as_str()),
    };
    let body = body.trim().to_owned();
    // An Obsidian note keeps its id when the user (or `names: title`) names the file otherwise.
    let id = match front.id.filter(|id| scope == Scope::Obsidian && is_id(id)) {
        Some(id) => id,
        None => stem,
    };
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

/// The memory with `id` in `dir`: by file name, and in a vault also by the id property.
fn find_in(dir: &Path, scope: Scope, id: &str) -> Option<Memory> {
    read(&dir.join(format!("{id}.md")), scope)
        .filter(|m| m.id == id)
        .or_else(|| (scope == Scope::Obsidian).then(|| load_dir(dir, scope).into_iter().find(|m| m.id == id)).flatten())
}

fn read_front(path: &Path) -> Option<Front> {
    let text = std::fs::read_to_string(path).ok()?.replace("\r\n", "\n");
    let (yaml, _) = split_front_matter(text.strip_prefix('\u{feff}').unwrap_or(&text))?;
    serde_yaml_ng::from_str(yaml).ok()
}

/// A file name for `title` that every system and Obsidian accept, numbered
/// when taken (ignoring case, as macOS and Windows do); `fallback` when
/// nothing of the title is usable.
fn unique_name(dir: &Path, title: &str, fallback: &str) -> String {
    let clean: String =
        title.chars().map(|c| if c.is_control() || r#"\/:*?"<>|#^[]"#.contains(c) { ' ' } else { c }).collect();
    let mut base = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    base = base.chars().take(MAX_NAME_CHARS).collect::<String>().trim_matches([' ', '.']).to_owned();
    let upper = base.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.ends_with(|c: char| c.is_ascii_digit()));
    if base.is_empty() || reserved {
        fallback.clone_into(&mut base);
    }
    let taken: BTreeSet<String> = [dir.to_path_buf(), dir.join(FORGOTTEN_DIR)]
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
        .collect();
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&format!("{}.md", name.to_lowercase())) {
        name = format!("{base} {n}");
        n += 1;
    }
    name
}

/// Wikilinks to the memories this one is about: those whose title it
/// mentions, or that share at least two tags.
fn related_links(others: &[Memory], id: &str, title: &str, body: &str, tags: &[String]) -> Vec<String> {
    let text = format!("{title}\n{body}").to_lowercase();
    others
        .iter()
        .filter(|other| other.id != id)
        .filter(|other| {
            let mentioned =
                other.title.chars().count() >= MIN_MENTION_CHARS && text.contains(&other.title.to_lowercase());
            mentioned || other.tags.iter().filter(|tag| tags.contains(tag)).count() >= 2
        })
        .filter_map(|other| {
            let stem = other.path.file_stem()?.to_str()?;
            let label = other.title.replace(['[', ']', '|'], " ");
            Some(format!("[[{stem}|{}]]", label.trim()))
        })
        .take(MAX_RELATED)
        .collect()
}

static FILE_MENTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s`(\[])((?:[\w.-]+/)+[\w.-]+\.[A-Za-z0-9]{1,8})\b").expect("valid file pattern")
});

/// Relative file paths the text mentions (`src/auth.rs`), without repeats.
fn mentioned_files(body: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for found in FILE_MENTION.captures_iter(body) {
        let path = found[1].to_owned();
        if !path.starts_with("..") && !files.contains(&path) {
            files.push(path);
        }
        if files.len() == MAX_FILES {
            break;
        }
    }
    files
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
fn unique_id(dir: &Path, scope: Scope, title: &str, now: &str) -> String {
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
    let used = |id: &str| {
        let forgotten = dir.join(FORGOTTEN_DIR);
        dir.join(format!("{id}.md")).exists()
            || forgotten.join(format!("{id}.md")).exists()
            // Notes named after their title keep the id inside.
            || (scope == Scope::Obsidian
                && [dir, forgotten.as_path()].iter().any(|d| load_dir(d, scope).iter().any(|m| m.id == id)))
    };
    while used(&id) {
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
        MemoryDirs {
            private: root.join("private"),
            shared: root.join("shared"),
            global: Some(root.join("global")),
            obsidian: None,
            obsidian_names: ObsidianNames::Id,
        }
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
    fn obsidian_scope_writes_into_the_vault_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        // A note the user wrote by hand in Obsidian, with a wikilink.
        std::fs::create_dir_all(vault.join("Brokspace")).unwrap();
        std::fs::write(
            vault.join("Brokspace").join("idea.md"),
            "---\ntags:\n  - idea\n---\n\n# Cache plan\n\nSee [[Redis]].\n",
        )
        .unwrap();
        let dirs = MemoryDirs { obsidian: Some(vault.join("Brokspace")), ..dirs(tmp.path()) };

        let saved =
            dirs.save(Scope::Obsidian, NewMemory { kind: Kind::Decision, ..note("Use Redis", "ttl 5m") }).unwrap();
        assert_eq!(saved.scope, Scope::Obsidian);
        assert_eq!(saved.path.parent().unwrap(), vault.join("Brokspace"));
        assert_eq!(dirs.find("idea", None).unwrap().title, "Cache plan");
        assert_eq!(dirs.list().len(), 2);

        // Secrets never reach a vault that a cloud service may sync.
        let leaked =
            dirs.save(Scope::Obsidian, note("Keys", "token ghp_abcdefghijklmnopqrstuvwxyz0123456789")).unwrap();
        assert!(!leaked.body.contains("ghp_"), "{}", leaked.body);

        dirs.forget(&saved.id, Some(Scope::Obsidian)).unwrap();
        assert!(vault.join("Brokspace").join(FORGOTTEN_DIR).join(format!("{}.md", saved.id)).is_file());
    }

    #[test]
    fn vault_notes_keep_their_path_as_id_and_read_obsidian_properties() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("Reunião de produto.md");
        std::fs::write(
            &path,
            "---\ntags: \"checkout, #orders\"\naliases:\n  - Pedidos\n---\nSee [[Carrinho]]. token ghp_abcdefghijklmnopqrstuvwxyz0123456789\n",
        )
        .unwrap();
        let note = read_vault_note(&path, "Projetos/Reunião de produto.md").unwrap();
        assert_eq!(note.id, "Projetos/Reunião de produto");
        assert_eq!(note.title, "Reunião de produto");
        assert_eq!(note.tags, ["checkout", "orders", "Pedidos"]);
        assert!(note.body.contains("[[Carrinho]]") && note.body.contains("[secret]"));
    }

    fn vault_dirs(root: &Path, names: ObsidianNames) -> MemoryDirs {
        std::fs::create_dir_all(root.join("vault")).unwrap();
        MemoryDirs { obsidian: Some(root.join("vault/Brokspace")), obsidian_names: names, ..dirs(root) }
    }

    #[test]
    fn title_named_notes_upsert_by_id_even_after_a_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vault_dirs(tmp.path(), ObsidianNames::Title);
        let first = dirs.save(Scope::Obsidian, note("Use Redis: sessions?", "ttl 5m")).unwrap();
        assert_eq!(first.path.file_name().unwrap(), "Use Redis sessions.md");
        let again = dirs.save(Scope::Obsidian, note("Use Redis: sessions?", "other")).unwrap();
        assert_eq!(again.path.file_name().unwrap(), "Use Redis sessions 2.md");
        assert_ne!(again.id, first.id);

        // The user renames the file in Obsidian and adds a property.
        let renamed = first.path.with_file_name("Sessões.md");
        let text = std::fs::read_to_string(&first.path).unwrap().replacen("---\n", "---\nstatus: accepted\n", 1);
        std::fs::remove_file(&first.path).unwrap();
        std::fs::write(&renamed, text).unwrap();
        assert_eq!(dirs.find(&first.id, Some(Scope::Obsidian)).unwrap().path, renamed);
        let updated = dirs
            .save(Scope::Obsidian, NewMemory { id: Some(first.id.clone()), ..note("Use Redis", "ttl 10m") })
            .unwrap();
        assert_eq!((&updated.path, &updated.id, &updated.created), (&renamed, &first.id, &first.created));
        let text = std::fs::read_to_string(&renamed).unwrap();
        assert!(text.contains("status: accepted") && text.contains(&format!("- {}", first.id)), "{text}");

        dirs.forget(&first.id, Some(Scope::Obsidian)).unwrap();
        assert!(!renamed.exists() && renamed.parent().unwrap().join(".forgotten/Sessões.md").exists());
        assert!(dirs.find(&first.id, None).is_none());
    }

    #[test]
    fn notes_link_related_memories_and_list_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vault_dirs(tmp.path(), ObsidianNames::Title);
        dirs.save(Scope::Obsidian, note("Orders are idempotent", "per cart")).unwrap();
        let saved = dirs
            .save(
                Scope::Obsidian,
                note(
                    "Retry policy",
                    "Safe because orders are idempotent. See `src/retry.rs` and https://x.dev/a/b.html",
                ),
            )
            .unwrap();
        let text = std::fs::read_to_string(saved.path).unwrap();
        assert!(text.contains("[[Orders are idempotent|Orders are idempotent]]"), "{text}");
        assert!(text.contains("- src/retry.rs") && !text.contains("- x.dev"), "{text}");
    }

    #[test]
    fn export_copies_memories_once_keeping_ids_dates_and_masking() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vault_dirs(tmp.path(), ObsidianNames::Title);
        let private = dirs.save(Scope::Private, note("Use Redis", "ttl 5m")).unwrap();
        dirs.save(Scope::Shared, note("API is versioned", "v1")).unwrap();
        dirs.save(Scope::Private, NewMemory { kind: Kind::Session, ..note("chat", "x") }).unwrap();
        assert_eq!(dirs.export_to_obsidian().unwrap(), Export { written: 2, unchanged: 0 });
        assert_eq!(dirs.export_to_obsidian().unwrap(), Export { written: 0, unchanged: 2 });
        let copy = dirs.find(&private.id, Some(Scope::Obsidian)).unwrap();
        assert_eq!((copy.created, copy.updated, copy.title), (private.created, private.updated, "Use Redis".into()));
    }

    #[test]
    fn names_are_safe_on_every_system() {
        let tmp = tempfile::tempdir().unwrap();
        for (title, expected) in [("a/b\\c [x] #y", "a b c x y"), ("CON", "id"), ("...", "id"), ("com1.notes", "id")] {
            assert_eq!(unique_name(tmp.path(), title, "id"), expected, "{title}");
        }
    }

    #[test]
    fn obsidian_scope_needs_a_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let err = dirs(tmp.path()).save(Scope::Obsidian, note("x", "y")).unwrap_err().to_string();
        assert!(err.contains("memory.obsidian.vault"), "{err}");
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
