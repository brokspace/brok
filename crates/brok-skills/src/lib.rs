//! Agent skills for Brokspace.
//!
//! A skill is a folder holding a `SKILL.md` (the Agent Skills format used by
//! Claude Code, Codex, Cursor, Copilot and others) plus optional reference
//! files. Brokspace resolves skills from three places, first match wins:
//!
//! 1. the workspace: `.brokspace/skills/<name>/`;
//! 2. installed shards that declare `"skills": [...]` in `shard.json`;
//! 3. the built-in catalog embedded in the binary (`skills/engineering`,
//!    `skills/productivity`).
//!
//! Skills reach coding agents through [`Skill::export_to`] (`brok skill
//! export`) and the local LLM through `brok skill run`.

mod skill;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brok_core::Workspace;
use include_dir::{Dir, include_dir};

pub use skill::{SKILL_FILE, Skill, SkillSource, is_valid_name};

/// File dropped in exported skill folders so brok only ever replaces its
/// own, and never indexes them.
pub use brok_core::fsutil::MANAGED_MARKER;

/// Built-in buckets embedded in the binary (`in-progress/` is not shipped).
pub const BUCKETS: [&str; 2] = ["engineering", "productivity"];

static BUILTIN: Dir<'static> = include_dir!("$BROK_BUILTIN_SKILLS");

/// The resolved set of skills for a workspace.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    skills: BTreeMap<String, Skill>,
    shadowed: Vec<(String, SkillSource)>,
    errors: Vec<String>,
}

impl Catalog {
    /// Built-in skills only.
    pub fn builtin() -> Self {
        let mut catalog = Self::default();
        catalog.add_builtin();
        catalog
    }

    /// Workspace, shard and built-in skills, with that precedence.
    pub fn load(workspace: &Workspace) -> Self {
        let mut catalog = Self::default();
        catalog.add_dir(&workspace_skills_dir(workspace), &SkillSource::Workspace);
        match brok_shard::Registry::new(workspace.paths()).list() {
            Ok(shards) => {
                for shard in shards {
                    let source = SkillSource::Shard(shard.manifest.name.clone());
                    for dir in shard.skill_dirs() {
                        catalog.add_dir(&dir, &source);
                    }
                }
            }
            Err(e) => catalog.errors.push(format!("cannot list shards: {e:#}")),
        }
        catalog.add_builtin();
        catalog
    }

    /// Skills sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = &Skill> {
        self.skills.values()
    }

    /// Number of skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Looks a skill up by name.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// Looks a skill up, with a "did you mean" hint on failure.
    pub fn require(&self, name: &str) -> Result<&Skill> {
        self.get(name).with_context(|| {
            let close: Vec<&str> = self
                .skills
                .keys()
                .filter(|k| k.contains(name) || name.contains(k.as_str()))
                .map(String::as_str)
                .collect();
            if close.is_empty() {
                format!("no skill named `{name}` (see `brok skill list`)")
            } else {
                format!("no skill named `{name}`; did you mean {}?", close.join(", "))
            }
        })
    }

    /// Skills hidden by a higher-precedence skill of the same name.
    pub fn shadowed(&self) -> &[(String, SkillSource)] {
        &self.shadowed
    }

    /// Skills that failed to load (bad frontmatter, unreadable files).
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    fn insert(&mut self, skill: Skill) {
        if self.skills.contains_key(&skill.name) {
            self.shadowed.push((skill.name, skill.source));
        } else {
            self.skills.insert(skill.name.clone(), skill);
        }
    }

    fn add_dir(&mut self, dir: &Path, source: &SkillSource) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut folders: Vec<PathBuf> =
            entries.flatten().map(|e| e.path()).filter(|p| p.join(SKILL_FILE).is_file()).collect();
        folders.sort();
        for folder in folders {
            match Skill::load(&folder, source.clone()) {
                Ok(skill) => self.insert(skill),
                Err(e) => self.errors.push(format!("{e:#}")),
            }
        }
    }

    fn add_builtin(&mut self) {
        for bucket in BUCKETS {
            let Some(bucket_dir) = BUILTIN.get_dir(bucket) else { continue };
            for dir in bucket_dir.dirs() {
                let Some(text) = dir.get_file(dir.path().join(SKILL_FILE)).and_then(|f| f.contents_utf8()) else {
                    continue;
                };
                let folder = dir.path().file_name().and_then(|n| n.to_str()).unwrap_or_default();
                match Skill::parse(folder, text, SkillSource::Builtin, skill::Files::Embedded(dir)) {
                    Ok(mut skill) => {
                        skill.bucket = Some(bucket.to_owned());
                        self.insert(skill);
                    }
                    Err(e) => self.errors.push(format!("builtin {bucket}/{folder}: {e:#}")),
                }
            }
        }
    }
}

/// Where workspace-specific skills live.
pub fn workspace_skills_dir(workspace: &Workspace) -> PathBuf {
    workspace.paths().brokspace.join("skills")
}

/// Creates a new skill folder `<parent>/<name>/` with a commented template.
pub fn scaffold(parent: &Path, name: &str, user_invoked: bool) -> Result<PathBuf> {
    if !is_valid_name(name) {
        bail!("`{name}` is not a valid skill name (lowercase letters, digits and dashes)");
    }
    let dir = parent.join(name);
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    std::fs::create_dir_all(dir.join("agents"))?;
    let (invocation, description) = if user_invoked {
        ("disable-model-invocation: true\n", format!("One line telling a human what /{name} does."))
    } else {
        ("", format!("What {name} does. Use when the user asks for..., mentions..., or wants...."))
    };
    let title = name.split('-').map(capitalize).collect::<Vec<_>>().join(" ");
    let skill = format!(
        "---\nname: {name}\ndescription: \"{description}\"\n{invocation}---\n\n# {title}\n\n\
         One or two sentences: the outcome this skill produces and when it applies.\n\n\
         ## Steps\n\n\
         1. First concrete action (commands, files to read, questions to ask).\n\
         2. Next action.\n\
         3. How to know you are done.\n\n\
         ## Rules\n\n\
         - Keep instructions short and explicit; small local models follow them too.\n\
         - Put long reference material in a sibling file and link it, e.g. [REFERENCE.md](REFERENCE.md).\n"
    );
    std::fs::write(dir.join(SKILL_FILE), skill)?;
    let mut openai =
        format!("interface:\n  display_name: \"{title}\"\n  short_description: \"Describe {name} in a few words\"\n");
    if user_invoked {
        openai.push_str("policy:\n  allow_implicit_invocation: false\n");
    }
    std::fs::write(dir.join("agents").join("openai.yaml"), openai)?;
    Ok(dir)
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid() {
        let catalog = Catalog::builtin();
        assert!(catalog.errors().is_empty(), "{:#?}", catalog.errors());
        assert!(catalog.len() >= 10, "expected the built-in skills, got {}", catalog.len());
        for name in ["brok-context", "tdd", "code-review", "grill-me", "setup-brokspace"] {
            assert!(catalog.get(name).is_some(), "built-in `{name}` missing");
        }
        assert!(catalog.get("setup-brokspace").unwrap().user_invoked);
        assert!(!catalog.get("tdd").unwrap().user_invoked);
    }

    #[test]
    fn workspace_skills_override_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = brok_core::init::init_workspace(dir.path(), Default::default()).unwrap().workspace;
        let skills = workspace_skills_dir(&workspace);
        let tdd = scaffold(&skills, "tdd", false).unwrap();
        assert!(tdd.join("agents/openai.yaml").is_file());
        scaffold(&skills, "deploy", true).unwrap();
        std::fs::create_dir_all(skills.join("broken")).unwrap();
        std::fs::write(skills.join("broken/SKILL.md"), "nope").unwrap();

        let catalog = Catalog::load(&workspace);
        assert_eq!(catalog.require("tdd").unwrap().source, SkillSource::Workspace);
        assert!(catalog.require("deploy").unwrap().user_invoked);
        assert!(catalog.shadowed().iter().any(|(name, source)| name == "tdd" && *source == SkillSource::Builtin));
        assert_eq!(catalog.errors().len(), 1);
        assert!(catalog.require("deplo").unwrap_err().to_string().contains("deploy"));
        assert!(scaffold(&skills, "tdd", false).is_err());
        assert!(scaffold(&skills, "Bad_Name", false).is_err());
    }
}
