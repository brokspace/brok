//! `brok review` and `brok commit`: the built-in skills applied to your git
//! changes, on your own LLM.

use std::path::Path;

use anyhow::{Context, Result, bail};
use brok_core::Workspace;
use brok_sdk::payload::{AskRequest, SkillInvocation};
use brok_skills::Catalog;
use dialoguer::Select;

use super::ask::{self, Output};
use crate::{daemon, ui};

/// Prompt budget: the agent caps prompts at 16k characters.
const PROMPT_BUDGET: usize = 14_000;
/// Space kept for the `--stat` summary of large changes.
const MAX_STAT_CHARS: usize = 2_000;

/// Arguments of `brok review`.
#[derive(Debug, clap::Args)]
pub struct ReviewArgs {
    /// Review only staged changes.
    #[arg(long)]
    staged: bool,
    /// Review the branch against this ref (e.g. `main`) instead of local changes.
    #[arg(long, conflicts_with = "staged")]
    base: Option<String>,
    /// LLM provider from `llm.providers` in brok.yml.
    #[arg(short, long)]
    provider: Option<String>,
}

/// Arguments of `brok commit`.
#[derive(Debug, clap::Args)]
pub struct CommitArgs {
    /// Commit without asking.
    #[arg(short, long)]
    yes: bool,
    /// Only print the suggested message.
    #[arg(long, conflicts_with = "yes")]
    print: bool,
    /// LLM provider from `llm.providers` in brok.yml.
    #[arg(short, long)]
    provider: Option<String>,
}

pub async fn review(cwd: &Path, args: ReviewArgs) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let (diff_args, what): (Vec<String>, String) = match &args.base {
        Some(base) => (vec!["diff".into(), format!("{base}...HEAD")], format!("this branch against {base}")),
        None if args.staged => (vec!["diff".into(), "--cached".into()], "the staged changes".into()),
        None => (vec!["diff".into(), "HEAD".into()], "the uncommitted changes".into()),
    };
    let diff = git(cwd, &diff_args)?;
    if diff.trim().is_empty() {
        ui::step(&format!("Nothing to review: {what} are empty."));
        return Ok(());
    }
    let (diff, truncated) = truncate(&diff, PROMPT_BUDGET - 600);
    let prompt = review_prompt(&what, diff, truncated);

    let skill = skill(&workspace, "code-review")?;
    let (client, _inbox) = daemon::connect_or_start(&workspace).await?;
    let request = AskRequest {
        prompt,
        provider: args.provider,
        context_chunks: Some(4),
        skill: Some(skill),
        history: Vec::new(),
    };
    ask::stream(&client, &request).await.map(drop)
}

pub async fn commit(cwd: &Path, args: CommitArgs) -> Result<()> {
    let workspace = super::workspace(cwd)?;
    let diff = git(cwd, &["diff".into(), "--cached".into()])?;
    if diff.trim().is_empty() {
        bail!("nothing is staged: `git add <files>` (or `git add -A`) first");
    }
    let stat = git(cwd, &["diff".into(), "--cached".into(), "--stat".into()])?;
    let (stat, _) = truncate(&stat, MAX_STAT_CHARS);
    let (diff, _) = truncate(&diff, PROMPT_BUDGET - stat.len() - 200);
    let prompt = format!("Write the commit message for this staged change.\n\n{stat}\n\n```diff\n{diff}\n```");

    let skill = skill(&workspace, "commit-message")?;
    let (client, _inbox) = daemon::connect_or_start(&workspace).await?;
    let request = AskRequest {
        prompt,
        provider: args.provider,
        context_chunks: Some(0),
        skill: Some(skill),
        history: Vec::new(),
    };
    let spinner = ui::spinner("Writing the commit message");
    let answer = ask::answer(&client, &request, Output::Silent).await;
    spinner.finish_and_clear();
    let message = clean_message(&answer?);
    if message.is_empty() {
        bail!("the model returned an empty message");
    }

    println!("{}\n", console::style(&message).bold());
    if args.print {
        return Ok(());
    }
    let action = if args.yes {
        0
    } else if ui::interactive() {
        Select::new()
            .with_prompt("Commit with this message?")
            .items(["Commit", "Edit in your git editor, then commit", "Cancel"])
            .default(0)
            .interact()?
    } else {
        bail!("not a terminal: pass --yes to commit or --print to only print the message");
    };
    if action == 2 {
        ui::step("Cancelled");
        return Ok(());
    }

    let file = tempfile_path(&workspace);
    std::fs::write(&file, format!("{message}\n"))?;
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(cwd).arg("commit").arg("-F").arg(&file);
    if action == 1 {
        command.arg("--edit");
    }
    let status = command.status().context("cannot run git")?;
    let _ = std::fs::remove_file(&file);
    if !status.success() {
        bail!("git commit failed");
    }
    Ok(())
}

/// The diff first and the instruction last: small local models follow the
/// end of a prompt best, and without "you have everything" they tend to ask
/// for the spec the skill mentions instead of reviewing.
fn review_prompt(what: &str, diff: &str, truncated: bool) -> String {
    let mut prompt = format!("These are {what}:\n\n```diff\n{diff}\n```\n\n");
    if truncated {
        prompt.push_str("The diff was truncated; say which parts you could not see.\n\n");
    }
    prompt.push_str(
        "Review this diff now. You have what you need: the diff and the workspace context. There is no \
         separate spec, so judge the code itself and check that the change is complete and consistent \
         with the code around it (for example, every place that must change together). Answer in the \
         skill's report format: findings with path:line, most severe first, then the one-line verdict. \
         Do not ask questions.",
    );
    prompt
}

fn skill(workspace: &Workspace, name: &str) -> Result<SkillInvocation> {
    let catalog = Catalog::load(workspace);
    let skill = catalog.require(name)?;
    Ok(SkillInvocation { name: skill.name.clone(), instructions: skill.instructions_with_references()? })
}

fn git(dir: &Path, args: &[String]) -> Result<String> {
    let output = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().context("cannot run git")?;
    if !output.status.success() {
        bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Cuts at a line boundary so the model never sees half a hunk header.
fn truncate(diff: &str, max: usize) -> (&str, bool) {
    if diff.len() <= max {
        return (diff, false);
    }
    let mut cut = max;
    while !diff.is_char_boundary(cut) {
        cut -= 1;
    }
    let cut = diff[..cut].rfind('\n').unwrap_or(cut);
    (&diff[..cut], true)
}

/// Strips what small models like to add around the message.
fn clean_message(answer: &str) -> String {
    let mut text = answer.trim();
    if let Some(inner) = text.strip_prefix("```") {
        let inner = inner.split_once('\n').map_or(inner, |(_, rest)| rest);
        text = inner.strip_suffix("```").unwrap_or(inner).trim();
    }
    text.trim_matches(['"', '\'', '`']).trim().to_owned()
}

fn tempfile_path(workspace: &Workspace) -> std::path::PathBuf {
    workspace.paths().run.join(format!("commit-message-{}.txt", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_model_noise() {
        assert_eq!(clean_message("```\nfeat(cli): add chat\n```"), "feat(cli): add chat");
        assert_eq!(clean_message("```text\nfix: x\n\nbody\n```\n"), "fix: x\n\nbody");
        assert_eq!(clean_message("\"docs: readme\""), "docs: readme");
        assert_eq!(clean_message("  chore: tidy  "), "chore: tidy");
    }

    #[test]
    fn review_prompt_ends_with_the_instruction() {
        let prompt = review_prompt("the staged changes", "+x", true);
        assert!(prompt.starts_with("These are the staged changes:\n\n```diff\n+x\n```"));
        assert!(prompt.contains("truncated"));
        assert!(prompt.ends_with("Do not ask questions."));
        // `review` reserves 600 characters of the budget for everything but the diff.
        assert!(review_prompt("this branch against origin/main", "", true).len() < 600);
    }

    #[test]
    fn truncates_at_line_boundaries() {
        let diff = "+line\n".repeat(5_000);
        let (cut, truncated) = truncate(&diff, PROMPT_BUDGET);
        assert!(truncated);
        assert!(cut.len() <= PROMPT_BUDGET);
        assert!(cut.ends_with("+line"));
        assert_eq!(truncate("small", 10), ("small", false));
    }
}
