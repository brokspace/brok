//! One skill: a folder with a `SKILL.md` and optional reference files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use include_dir::Dir;
use serde::Deserialize;

/// The skill's entry file.
pub const SKILL_FILE: &str = "SKILL.md";
/// Upper bound for `instructions_with_references` (a few thousand tokens).
const INSTRUCTIONS_BUDGET: usize = 24_000;

/// Where a skill comes from. Earlier variants win when names collide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSource {
    /// `.brokspace/skills/<name>/` in the workspace.
    Workspace,
    /// An installed shard.
    Shard(String),
    /// Embedded in the `brok` binary.
    Builtin,
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workspace => f.write_str("workspace"),
            Self::Shard(name) => write!(f, "shard:{name}"),
            Self::Builtin => f.write_str("builtin"),
        }
    }
}

/// Where the skill's files live.
#[derive(Debug, Clone)]
pub(crate) enum Files {
    Embedded(&'static Dir<'static>),
    Disk(PathBuf),
}

/// A parsed skill.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Unique name (equals the folder name).
    pub name: String,
    /// One-line description from the frontmatter.
    pub description: String,
    /// `true` when only a human may invoke it (`disable-model-invocation`).
    pub user_invoked: bool,
    /// Bucket (`engineering`, `productivity`) for built-ins.
    pub bucket: Option<String>,
    /// Provenance.
    pub source: SkillSource,
    /// Markdown body without the frontmatter.
    pub body: String,
    pub(crate) files: Files,
}

#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default, rename = "disable-model-invocation")]
    disable_model_invocation: bool,
}

impl Skill {
    /// Parses `SKILL.md` text. `folder` is the skill folder name, which must
    /// match the `name` field.
    pub(crate) fn parse(folder: &str, text: &str, source: SkillSource, files: Files) -> Result<Self> {
        let text = text.trim_start_matches('\u{feff}');
        let rest = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"))
            .context("SKILL.md must start with a `---` YAML frontmatter block")?;
        let end = rest.find("\n---").context("unterminated frontmatter (missing closing `---`)")?;
        let frontmatter: Frontmatter = serde_yaml_ng::from_str(&rest[..end]).context("invalid SKILL.md frontmatter")?;
        let body = rest[end + 4..].trim_start_matches(['-', '\r', '\n']).to_owned();

        ensure!(is_valid_name(&frontmatter.name), "skill name `{}` must be lowercase kebab-case", frontmatter.name);
        ensure!(frontmatter.name == folder, "skill name `{}` must match its folder `{folder}`", frontmatter.name);
        ensure!(!frontmatter.description.trim().is_empty(), "skill `{folder}` has an empty description");
        Ok(Self {
            name: frontmatter.name,
            description: frontmatter.description.trim().to_owned(),
            user_invoked: frontmatter.disable_model_invocation,
            bucket: None,
            source,
            body,
            files,
        })
    }

    /// Loads a skill from a folder on disk.
    pub fn load(dir: &Path, source: SkillSource) -> Result<Self> {
        let folder = dir.file_name().and_then(|n| n.to_str()).context("invalid skill folder")?;
        let text = std::fs::read_to_string(dir.join(SKILL_FILE))
            .with_context(|| format!("reading {}", dir.join(SKILL_FILE).display()))?;
        Self::parse(folder, &text, source, Files::Disk(dir.to_path_buf()))
            .with_context(|| format!("in {}", dir.display()))
    }

    /// Every file of the skill as (relative path with `/`, bytes).
    pub fn files(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        match &self.files {
            Files::Embedded(dir) => collect_embedded(dir, dir.path(), &mut out),
            Files::Disk(dir) => collect_disk(dir, dir, &mut out)?,
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// The instructions handed to a model by `brok skill run`: the body,
    /// followed by the reference files it links to (within a size budget).
    pub fn instructions_with_references(&self) -> Result<String> {
        let mut text = format!("# Skill: {}\n\n{}", self.name, self.body.trim());
        for (path, bytes) in self.files()? {
            let referenced = path.ends_with(".md") && path != SKILL_FILE && self.body.contains(file_name(&path));
            if !referenced {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            if text.len() + content.len() > INSTRUCTIONS_BUDGET {
                tracing::debug!(skill = %self.name, file = %path, "reference file left out (size budget)");
                continue;
            }
            text.push_str(&format!("\n\n---\n\n## Reference: {path}\n\n{}", content.trim()));
        }
        Ok(text)
    }

    /// Writes the skill's files into `<target>/<name>/`, replacing what was
    /// there, and marks the folder as managed by brok.
    pub fn export_to(&self, target: &Path) -> Result<PathBuf> {
        let dir = target.join(&self.name);
        if dir.exists() {
            if !dir.join(crate::MANAGED_MARKER).exists() {
                bail!("{} exists and was not created by brok; remove it or pick another target", dir.display());
            }
            std::fs::remove_dir_all(&dir).with_context(|| format!("replacing {}", dir.display()))?;
        }
        for (path, bytes) in self.files()? {
            let file = dir.join(&path);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file, bytes).with_context(|| format!("writing {}", file.display()))?;
        }
        std::fs::write(
            dir.join(crate::MANAGED_MARKER),
            format!(
                "Exported by `brok skill export` from {}. Re-run it to update; edits here are overwritten.\n",
                self.source
            ),
        )?;
        Ok(dir)
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether `name` is lowercase kebab-case (1..=64 chars).
pub fn is_valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn collect_embedded(dir: &Dir<'_>, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for file in dir.files() {
        let rel = file.path().strip_prefix(root).unwrap_or(file.path());
        out.push((rel.to_string_lossy().replace('\\', "/"), file.contents().to_vec()));
    }
    for sub in dir.dirs() {
        collect_embedded(sub, root, out);
    }
}

fn collect_disk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.file_name().and_then(|n| n.to_str()) == Some(crate::MANAGED_MARKER) {
            continue;
        }
        if path.is_dir() {
            collect_disk(&path, root, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.push((rel.to_string_lossy().replace('\\', "/"), std::fs::read(&path)?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: code-review\ndescription: \"Review a diff: standards and spec.\"\ndisable-model-invocation: true\n---\n\n# Code review\n\nSee [CHECKLIST.md](CHECKLIST.md).\n";

    fn parse(folder: &str, text: &str) -> Result<Skill> {
        Skill::parse(folder, text, SkillSource::Workspace, Files::Disk(PathBuf::from(".")))
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let skill = parse("code-review", SAMPLE).unwrap();
        assert_eq!(skill.description, "Review a diff: standards and spec.");
        assert!(skill.user_invoked);
        assert!(skill.body.starts_with("# Code review"));
    }

    #[test]
    fn crlf_and_defaults() {
        let skill = parse("tdd", "---\r\nname: tdd\r\ndescription: Test first.\r\n---\r\nBody\r\n").unwrap();
        assert!(!skill.user_invoked);
        assert!(skill.body.starts_with("Body"));
    }

    #[test]
    fn rejects_bad_skills() {
        assert!(parse("other", SAMPLE).is_err(), "name must match folder");
        assert!(parse("x", "no frontmatter").is_err());
        assert!(parse("Bad", "---\nname: Bad\ndescription: d\n---\n").is_err());
        assert!(parse("x", "---\nname: x\ndescription: \"\"\n---\n").is_err());
    }

    #[test]
    fn export_and_references() {
        let src = tempfile::tempdir().unwrap();
        let dir = src.path().join("code-review");
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(dir.join(SKILL_FILE), SAMPLE).unwrap();
        std::fs::write(dir.join("CHECKLIST.md"), "- naming\n").unwrap();
        std::fs::write(dir.join("agents/openai.yaml"), "interface: {}\n").unwrap();
        let skill = Skill::load(&dir, SkillSource::Workspace).unwrap();

        let instructions = skill.instructions_with_references().unwrap();
        assert!(instructions.contains("## Reference: CHECKLIST.md"));
        assert!(!instructions.contains("interface"));

        let target = tempfile::tempdir().unwrap();
        let exported = skill.export_to(target.path()).unwrap();
        assert!(exported.join("agents/openai.yaml").is_file());
        assert!(exported.join(crate::MANAGED_MARKER).is_file());
        skill.export_to(target.path()).unwrap(); // idempotent

        let foreign = target.path().join("mine");
        std::fs::create_dir_all(&foreign).unwrap();
        let mut renamed = skill.clone();
        renamed.name = "mine".into();
        assert!(renamed.export_to(target.path()).is_err(), "never clobber user folders");
    }
}
