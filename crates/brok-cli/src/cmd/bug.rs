//! `brok bug`: a bug report with everything a maintainer needs to reproduce
//! it, and nothing private.
//!
//! The report gathers the versions, the OS, `brok doctor`, a summary of the
//! local metrics (`brok stats`), the end of the log and the configuration,
//! with secrets masked and the home directory shown as `~`. It is saved to a
//! file (to attach) and summarized into a prefilled GitHub issue that opens
//! in the browser: nothing is sent until the user submits the issue there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brok_core::Workspace;
use dialoguer::{Confirm, Input};
use serde_json::json;

use super::doctor::{self, Status};
use crate::{daemon, ui};

/// Where issues go.
const ORG: &str = "brokspace";
/// Repositories a report can go to.
const REPOS: [&str; 3] = ["brok", "brok-agent", "ide"];
/// Lines of the log in the report.
const LOG_LINES: usize = 60;
/// Characters of diagnostics put in the issue URL (GitHub limits URLs to about 8 KB).
const URL_DIAGNOSTICS: usize = 4_500;

/// Arguments of `brok bug`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// One line summary of the problem.
    #[arg(short, long)]
    title: Option<String>,
    /// What happened.
    #[arg(long)]
    what_happened: Option<String>,
    /// What you expected instead.
    #[arg(long)]
    expected: Option<String>,
    /// The commands or steps that show it.
    #[arg(long)]
    steps: Option<String>,
    /// Repository: brok (the CLI), brok-agent (answers, reviews) or ide.
    #[arg(long, default_value = "brok", value_parser = REPOS)]
    repo: String,
    /// More environment details (editor extensions add their versions here).
    #[arg(long, value_name = "TEXT")]
    context: Option<String>,
    /// Where to save the full report (default: .brokspace/reports/).
    #[arg(long, value_name = "FILE")]
    save: Option<PathBuf>,
    /// Print the issue link instead of opening the browser.
    #[arg(long)]
    no_open: bool,
    /// Do not ask anything.
    #[arg(short, long)]
    yes: bool,
    /// Print `{"repo", "url", "file", "report"}` and open nothing (for editors).
    #[arg(long)]
    json: bool,
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let ask = ui::interactive() && !args.yes && !args.json;
    let title = match args.title.clone() {
        Some(title) => title,
        None if ask => Input::<String>::new().with_prompt("In one line, what went wrong?").interact_text()?,
        None => String::new(),
    };
    let what_happened = match args.what_happened.clone() {
        Some(text) => text,
        None if ask => Input::<String>::new()
            .with_prompt("What happened? (Enter to fill it in on GitHub)")
            .allow_empty(true)
            .interact_text()?,
        None => String::new(),
    };

    let report = report(cwd, args.context.as_deref()).await;
    let file = save(cwd, args.save.as_deref(), &report)?;
    let url =
        issue_url(&args.repo, &title, &what_happened, args.expected.as_deref(), args.steps.as_deref(), &report, &file);

    if args.json {
        let shown = home_relative(&file.display().to_string());
        println!("{}", json!({ "repo": args.repo, "url": url, "file": file, "file_shown": shown, "report": report }));
        return Ok(());
    }

    println!("{}", report.trim_end());
    println!();
    ui::success(&format!("Report saved to {}", file.display()));
    ui::hint("secrets are masked and your home directory shows as ~; read it before you share it");
    if ask
        && !Confirm::new()
            .with_prompt(format!("Open a prefilled issue in brokspace/{}?", args.repo))
            .default(true)
            .interact()?
    {
        return Ok(());
    }
    if args.no_open || open_url(&url).is_err() {
        println!("\nOpen this link to file the issue (attach the report file):\n{url}");
    } else {
        ui::step("opened the issue form in your browser: review it, attach the report file, submit");
    }
    Ok(())
}

/// The full report, Markdown.
pub async fn report(cwd: &Path, context: Option<&str>) -> String {
    let mut out = String::from("## Environment\n\n");
    let features = brok_index::compiled_features();
    let os = sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned());
    let _ = writeln!(
        out,
        "- brok {} ({})\n- {os} · {}",
        env!("CARGO_PKG_VERSION"),
        if features.is_empty() { "lean build".to_owned() } else { features.join(", ") },
        std::env::consts::ARCH
    );
    if let Some(context) = context.filter(|c| !c.trim().is_empty()) {
        for line in context.lines().filter(|l| !l.trim().is_empty()) {
            let _ = writeln!(out, "- {}", line.trim().trim_start_matches("- "));
        }
    }
    let workspace = Workspace::discover(cwd).ok();
    if let Some(workspace) = &workspace {
        let config = workspace.config();
        let llm = config.llm.providers.get(&config.llm.default);
        let _ = writeln!(
            out,
            "- LLM: {} ({}, {})",
            llm.map_or("none", |p| p.model.as_str()),
            config.llm.default,
            llm.map_or("-", |p| if is_local(&p.base_url) { "local" } else { "hosted" })
        );
        let _ = writeln!(
            out,
            "- embeddings: {} · store: {:?} · links: {} · memory: {} · daemon: {}",
            if config.embeddings.enabled { config.embeddings.model.as_str() } else { "off" },
            config.index.store,
            config.workspace.links.len(),
            if config.memory.enabled { "on" } else { "off" },
            if daemon::is_running(workspace).await { "running" } else { "stopped" }
        );
    }

    out.push_str("\n## brok doctor\n\n```text\n");
    for check in doctor::checks(cwd).await {
        let mark = match check.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        let fix = check.fix.map(|f| format!("  (fix: {f})")).unwrap_or_default();
        let _ = writeln!(out, "{mark} {:<12} {}{fix}", check.name, check.detail);
    }
    out.push_str("```\n");

    if let Some(workspace) = &workspace {
        let summary = super::stats::summarize(&brok_core::metrics::load(workspace.paths(), 7), 7);
        out.push_str("\n## Last 7 days (brok stats)\n\n");
        let _ = writeln!(
            out,
            "- searches: {} (p50 {} ms, p95 {} ms, {} empty)\n- answers: {} completed (p50 {} ms), {} failed\n- index runs: {} (p50 {} ms)\n- errors: {}",
            summary.search.latency.count,
            summary.search.latency.p50_ms.unwrap_or(0),
            summary.search.latency.p95_ms.unwrap_or(0),
            summary.search.empty,
            summary.answers.latency.count,
            summary.answers.latency.p50_ms.unwrap_or(0),
            summary.answers.failed,
            summary.index.latency.count,
            summary.index.latency.p50_ms.unwrap_or(0),
            summary.errors.count
        );
        for failure in &summary.errors.recent {
            let _ = writeln!(out, "  - {} `{}`: {}", failure.at, failure.command, failure.message);
        }

        let log = std::fs::read_to_string(daemon::log_file(workspace)).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        if !lines.is_empty() {
            let _ = write!(
                out,
                "\n## Log (last {} lines of .brokspace/logs/brok.log)\n\n```text\n",
                LOG_LINES.min(lines.len())
            );
            for line in &lines[lines.len().saturating_sub(LOG_LINES)..] {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("```\n");
        }

        if let Ok(yaml) = serde_yaml_ng::to_string(workspace.config()) {
            out.push_str("\n## brok.yml\n\n```yaml\n");
            out.push_str(&yaml);
            out.push_str("```\n");
        }
    }
    redact(&out)
}

/// Masks secrets and shows the home directory as `~`.
pub fn redact(text: &str) -> String {
    home_relative(&brok_core::memory::mask_secrets(text))
}

fn home_relative(text: &str) -> String {
    let Some(home) = dirs_home() else { return text.to_owned() };
    let home = home.to_string_lossy().into_owned();
    if home.len() < 3 {
        return text.to_owned();
    }
    // Both separators: Windows paths appear with either.
    text.replace(&home, "~").replace(&home.replace('\\', "/"), "~")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Whether an LLM base URL points at this machine.
fn is_local(url: &str) -> bool {
    ["://localhost", "://127.", "://[::1]", "://0.0.0.0", "://host.docker.internal"].iter().any(|h| url.contains(h))
}

fn save(cwd: &Path, target: Option<&Path>, report: &str) -> Result<PathBuf> {
    let path = match target {
        Some(path) => path.to_path_buf(),
        None => {
            let stamp = brok_core::memory::now_rfc3339()
                .replace([':', '-'], "")
                .replace('T', "-")
                .trim_end_matches('Z')
                .to_owned();
            let dir = match Workspace::discover(cwd) {
                Ok(workspace) => workspace.paths().brokspace.join("reports"),
                Err(_) => std::env::temp_dir(),
            };
            dir.join(format!("brokspace-report-{stamp}.md"))
        }
    };
    brok_core::fsutil::write_atomic(&path, report.as_bytes()).with_context(|| format!("saving {}", path.display()))?;
    Ok(path)
}

/// A link to the repository's bug form, prefilled.
fn issue_url(
    repo: &str,
    title: &str,
    what_happened: &str,
    expected: Option<&str>,
    steps: Option<&str>,
    report: &str,
    file: &Path,
) -> String {
    let mut diagnostics = section(report, "## Environment", "## Log");
    if diagnostics.chars().count() > URL_DIAGNOSTICS {
        diagnostics = diagnostics.chars().take(URL_DIAGNOSTICS).collect::<String>() + "\n…";
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    diagnostics.push_str(&format!("\n\nFull report (log, configuration): attach `{name}`, saved by `brok bug`."));
    let mut params = vec![("template", "bug.yml".to_owned()), ("title", title.to_owned()), ("doctor", diagnostics)];
    for (key, value) in [("what-happened", Some(what_happened)), ("expected", expected), ("command", steps)] {
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            params.push((key, value.to_owned()));
        }
    }
    let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
    format!("https://github.com/{ORG}/{repo}/issues/new?{}", query.join("&"))
}

/// The part of `report` from `start` up to (not including) `end`.
fn section(report: &str, start: &str, end: &str) -> String {
    let from = report.find(start).unwrap_or(0);
    let to = report[from..].find(end).map_or(report.len(), |i| from + i);
    report[from..to].trim().to_owned()
}

/// Percent-encodes a query value.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Opens `url` in the default browser.
fn open_url(url: &str) -> Result<()> {
    let status = if cfg!(windows) {
        // `start` would read `&` in the URL as a command separator.
        std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).status()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).status()
    } else {
        std::process::Command::new("xdg-open").arg(url).status()
    };
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => bail!("the browser could not be opened ({status})"),
        Err(e) => bail!("the browser could not be opened: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_encoded_and_bounded() {
        let report =
            format!("## Environment\n\n- brok 0.4.0\n\n## brok doctor\n\n{}\n## Log\n\nsecret log", "x".repeat(10_000));
        let url = issue_url("ide", "Crash & burn", "it broke", None, Some("brok ask"), &report, Path::new("/tmp/r.md"));
        assert!(url.starts_with(
            "https://github.com/brokspace/ide/issues/new?template=bug.yml&title=Crash%20%26%20burn&doctor="
        ));
        assert!(url.contains("&what-happened=it%20broke&command=brok%20ask"), "{url}");
        assert!(!url.contains("expected="));
        assert!(!url.contains("secret%20log"), "the log stays out of the URL");
        assert!(url.len() < 7_000, "{}", url.len());
    }

    #[test]
    fn redaction() {
        let text = redact("api_key: sk-abcdefghijklmnopqrstuvwxyz123456 and token=supersecretvalue");
        assert!(!text.contains("abcdefghij") && !text.contains("supersecretvalue"), "{text}");
        if let Some(home) = dirs_home() {
            let path = home.join("code").join("app");
            assert!(redact(&path.display().to_string()).starts_with('~'));
        }
    }

    #[test]
    fn local_llms() {
        assert!(is_local("http://localhost:11434/v1"));
        assert!(is_local("http://127.0.0.1:1234/v1"));
        assert!(!is_local("https://api.anthropic.com/v1"));
    }
}
