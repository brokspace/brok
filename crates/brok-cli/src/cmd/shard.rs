//! `brok shard`

use std::path::Path;

use anyhow::{Context, Result, bail};
use brok_sdk::ShardManifest;
use brok_shard::{AutoApprove, Consent, InstallMethod, InstallOptions, InstallSource, Registry};
use console::style;

use crate::ui;

/// `brok shard` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Create a new shard project (from brok-shard-template, or skills only).
    New {
        /// Shard name (lowercase, digits and dashes).
        name: String,
        /// A skill shard: only `shard.json` and `skills/`, no code to build.
        #[arg(long)]
        skills_only: bool,
        /// Directory to create (default: ./<name>).
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
    /// Install a shard from a git repository or a local directory.
    Install {
        /// Git URL (https://, ssh://, git@…) or path to a shard directory.
        source: String,
        /// Tag or branch to install.
        #[arg(long)]
        tag: Option<String>,
        /// Approve the requested capabilities without asking.
        #[arg(short, long)]
        yes: bool,
        /// Reinstall even if the same version is installed.
        #[arg(long)]
        force: bool,
    },
    /// List installed shards.
    List,
    /// Show a shard's manifest and provenance.
    Info {
        /// Shard name.
        name: String,
    },
    /// Uninstall a shard.
    Remove {
        /// Shard name.
        name: String,
    },
}

/// Interactive consent prompt.
struct Prompt;

impl Consent for Prompt {
    fn approve(&self, manifest: &ShardManifest, report: &[String]) -> bool {
        println!("{} {} {}", style("Installing").bold(), style(&manifest.name).cyan().bold(), manifest.version);
        if !manifest.description.is_empty() {
            println!("  {}", style(&manifest.description).dim());
        }
        for line in report {
            let line = if line.starts_with("warning") { style(line).yellow().to_string() } else { line.clone() };
            println!("  {line}");
        }
        dialoguer::Confirm::new()
            .with_prompt("Grant these capabilities and install?")
            .default(false)
            .interact()
            .unwrap_or(false)
    }
}

pub async fn run(cwd: &Path, command: Command) -> Result<()> {
    if let Command::New { name, skills_only, dir } = command {
        let target = dir.unwrap_or_else(|| cwd.join(&name));
        return if skills_only { new_skill_shard(&name, &target) } else { new_process_shard(&name, &target).await };
    }
    let workspace = super::workspace(cwd)?;
    let registry = Registry::new(workspace.paths());
    match command {
        Command::Install { source, tag, yes, force } => {
            let consent: &dyn Consent = if yes {
                &AutoApprove
            } else if ui::interactive() {
                &Prompt
            } else {
                bail!("not a terminal: pass --yes to approve the shard's capabilities");
            };
            let source = InstallSource::parse(&source, tag);
            let outcome =
                brok_shard::install(&registry, source, InstallOptions { force }, consent, &|line| ui::step(line))
                    .await?;
            let manifest = &outcome.shard.manifest;
            let how = match outcome.method {
                InstallMethod::Release => "verified release",
                InstallMethod::Build => "built from source",
                InstallMethod::Prebuilt => "prebuilt",
                InstallMethod::SkillsOnly => "skills",
            };
            match outcome.replaced {
                Some(old) if old == manifest.version => {
                    ui::success(&format!("Reinstalled {} {old} ({how})", manifest.name))
                }
                Some(old) => ui::success(&format!("Upgraded {} {old} → {} ({how})", manifest.name, manifest.version)),
                None => ui::success(&format!("Installed {} {} ({how})", manifest.name, manifest.version)),
            }
            if manifest.has_process() {
                ui::hint("Restart `brok run` to start it.");
            }
            if !manifest.skills.is_empty() {
                ui::hint(
                    "Its skills are available now: `brok skill list`; refresh your agents with `brok skill export`.",
                );
            }
        }
        Command::List => {
            let shards = registry.list()?;
            if shards.is_empty() {
                println!("No shards installed.");
            }
            for shard in shards {
                let source = shard.lock.as_ref().map(|l| l.source.to_string()).unwrap_or_default();
                println!(
                    "{:<24} {:<10} {}",
                    style(&shard.manifest.name).cyan(),
                    shard.manifest.version,
                    style(source).dim()
                );
            }
        }
        Command::Info { name } => {
            let Some(shard) = registry.get(&name)? else { bail!("shard `{name}` is not installed") };
            println!("{}", shard.manifest.to_json_pretty());
            if let Some(lock) = &shard.lock {
                println!();
                println!("source:  {}", lock.source);
                println!("sha256:  {}", lock.binary_sha256);
            }
            match shard.executable() {
                Some(path) => println!("binary:  {}", path.display()),
                None => println!("binary:  none (skill-only shard)"),
            }
            for dir in shard.skill_dirs() {
                println!("skills:  {}", dir.display());
            }
        }
        Command::New { .. } => unreachable!("handled above"),
        Command::Remove { name } => {
            if registry.remove(&name)? {
                ui::success(&format!("Removed {name}"));
            } else {
                bail!("shard `{name}` is not installed");
            }
        }
    }
    Ok(())
}

const TEMPLATE_URL: &str = "https://github.com/brokspace/brok-shard-template";

fn check_new_target(name: &str, target: &Path) -> Result<()> {
    if !brok_sdk::manifest::is_valid_name(name) {
        bail!("`{name}` is not a valid shard name (lowercase letters, digits and dashes, 2 to 64 chars)");
    }
    if target.exists() {
        bail!("{} already exists", target.display());
    }
    Ok(())
}

/// A shard that only ships agent skills: no build, no process.
fn new_skill_shard(name: &str, target: &Path) -> Result<()> {
    check_new_target(name, target)?;
    std::fs::create_dir_all(target)?;
    let manifest = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/brokspace/brok-sdk/main/shard.schema.json",
        "name": name,
        "version": "0.1.0",
        "description": format!("Agent skills from {name}"),
        "license": "MIT",
        "brok": ">=0.1.0",
        "skills": ["skills"],
    });
    let json = serde_json::to_string_pretty(&manifest)?
        + "
";
    ShardManifest::from_json(&json)?;
    std::fs::write(target.join("shard.json"), json)?;
    brok_skills::scaffold(&target.join("skills"), "example", false)?;
    std::fs::write(
        target.join("README.md"),
        format!(
            "# {name}

Agent skills for [Brokspace](https://github.com/brokspace/brok), installable as a skill shard.

             ## Use

```sh
brok shard install https://github.com/<you>/{name}
brok skill list
             brok skill export          # into Claude Code, Cursor, Codex, Copilot...
```

             ## Develop

```sh
brok skill new my-skill --dir skills   # add a skill
             brok shard install ./ --force           # install your working copy
             brok skill run example \"...\"           # try it on the local LLM (needs brok run)
```

             Each folder in `skills/` is one skill: a `SKILL.md` with `name` and `description` frontmatter.
             Guide: https://github.com/brokspace/brok/blob/main/docs/skills.md
"
        ),
    )?;
    ui::success(&format!("Created skill shard {}", target.display()));
    ui::hint(&format!("Edit skills/example/SKILL.md, then: brok shard install {}", target.display()));
    Ok(())
}

/// A shard with a process, copied from the official template.
async fn new_process_shard(name: &str, target: &Path) -> Result<()> {
    check_new_target(name, target)?;
    ui::step(&format!("Cloning {TEMPLATE_URL}"));
    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", TEMPLATE_URL])
        .arg(target)
        .output()
        .await
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "cloning the template failed: {}
Offline? `brok shard new {name} --skills-only` needs no network.",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    std::fs::remove_dir_all(target.join(".git"))?;
    let _ = std::fs::remove_file(target.join(".github").join("CODEOWNERS"));
    rename_in_tree(target, name)?;
    let _ = tokio::process::Command::new("git").arg("init").arg("--quiet").arg(target).status().await;
    ShardManifest::load_from_dir(target)?;
    ui::success(&format!("Created shard {}", target.display()));
    ui::hint("Implement your handler in src/main.rs, declare topics and permissions in shard.json, then:");
    ui::hint(&format!("brok shard install {}   &&   brok run   &&   brok bus tail", target.display()));
    Ok(())
}

/// Replaces the template's name everywhere (text files only).
fn rename_in_tree(dir: &Path, name: &str) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target" || n == ".git") {
                continue;
            }
            rename_in_tree(&path, name)?;
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            let renamed = text
                .replace("brokspace/brok-shard-template", &format!("OWNER/{name}"))
                .replace("brok-shard-template", name)
                .replace("brok_shard_template", &name.replace('-', "_"));
            if renamed != text {
                std::fs::write(&path, renamed)?;
            }
        }
    }
    Ok(())
}
