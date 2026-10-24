//! `brok doctor`: checks everything Brokspace needs and says how to fix it.

use std::path::Path;

use anyhow::{Result, bail};
use brok_core::Workspace;
use brok_index::{Access, Index, ModelFiles};
use brok_shard::{Registry, SYSTEM_AGENT};
use console::style;

use crate::integrations::{self, Agent};
use crate::{daemon, local_llm};

/// Arguments of `brok doctor`.
#[derive(Debug, clap::Args)]
pub struct Args {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

struct Report {
    failures: usize,
    warnings: usize,
}

impl Report {
    fn line(&mut self, status: Status, name: &str, detail: &str, fix: Option<&str>) {
        let mark = match status {
            Status::Ok => style("✓").green().bold(),
            Status::Warn => {
                self.warnings += 1;
                style("!").yellow().bold()
            }
            Status::Fail => {
                self.failures += 1;
                style("✗").red().bold()
            }
        };
        println!("{mark} {:<14} {detail}", style(name).bold());
        if let Some(fix) = fix {
            println!("  {:<14} {}", "", style(format!("fix: {fix}")).dim());
        }
    }
}

pub async fn run(cwd: &Path, _args: Args) -> Result<()> {
    let mut report = Report { failures: 0, warnings: 0 };
    let features = brok_index::compiled_features();
    let build = if features.is_empty() { "lean build: text search only".to_owned() } else { features.join(", ") };
    report.line(Status::Ok, "brok", &format!("{} ({build})", env!("CARGO_PKG_VERSION")), None);

    if integrations::on_path("git") {
        report.line(Status::Ok, "git", "found", None);
    } else {
        report.line(Status::Fail, "git", "not found on PATH", Some("install git (https://git-scm.com)"));
    }
    if !integrations::on_path("brok") {
        report.line(
            Status::Warn,
            "PATH",
            "`brok` is not on PATH",
            Some("add the folder containing brok to PATH (coding agents start `brok mcp` by name)"),
        );
    }

    match Workspace::discover(cwd) {
        Ok(workspace) => check_workspace(&workspace, &mut report).await,
        Err(e) => report.line(Status::Fail, "workspace", &format!("{e}"), Some("run `brok init` in your project root")),
    }

    println!();
    match (report.failures, report.warnings) {
        (0, 0) => println!("{}", style("Everything looks good.").green()),
        (0, w) => println!("{}", style(format!("{w} suggestion(s); Brokspace works, these unlock more.")).yellow()),
        (f, _) => bail!("{f} problem(s) need attention"),
    }
    Ok(())
}

async fn check_workspace(workspace: &Workspace, report: &mut Report) {
    let config = workspace.config();
    report.line(Status::Ok, "workspace", &workspace.root().display().to_string(), None);

    let index_status = match Index::open(workspace, None, Access::ReadOnly).await {
        Ok(index) => index.status().await,
        Err(e) => Err(e),
    };
    match index_status {
        Ok(parts) if parts.iter().any(|p| p.chunks > 0) => {
            let files: usize = parts.iter().map(|p| p.files).sum();
            let chunks: usize = parts.iter().map(|p| p.chunks).sum();
            let mode = if parts.iter().any(|p| p.semantic) { "hybrid" } else { "text only" };
            report.line(Status::Ok, "index", &format!("{files} files, {chunks} chunks ({mode})"), None);
        }
        Ok(_) => report.line(Status::Warn, "index", "empty", Some("brok index")),
        Err(e) => report.line(Status::Fail, "index", &format!("{e:#}"), Some("brok index --rebuild")),
    }

    if config.embeddings.enabled {
        match ModelFiles::locate(&workspace.paths().models, &config.embeddings) {
            Ok(files) if files.is_present() && brok_index::compiled_features().contains(&"onnx") => {
                report.line(Status::Ok, "embeddings", &config.embeddings.model, None)
            }
            Ok(files) if files.is_present() => report.line(
                Status::Warn,
                "embeddings",
                "model present but this build has no ONNX support",
                Some("install the full build, or set embeddings.enabled: false"),
            ),
            Ok(_) => report.line(Status::Warn, "embeddings", "model not downloaded", Some("brok model download")),
            Err(e) => report.line(Status::Fail, "embeddings", &format!("{e:#}"), Some("fix brok.yml embeddings.model")),
        }
    } else {
        report.line(Status::Ok, "embeddings", "disabled (text search only)", None);
    }

    match config.llm.providers.get(&config.llm.default) {
        Some(provider) => {
            let name = &config.llm.default;
            let missing_key = provider.api_key_env.as_ref().filter(|var| std::env::var_os(var).is_none());
            if let Some(var) = missing_key {
                report.line(
                    Status::Warn,
                    "llm",
                    &format!("{name}: {var} is not set"),
                    Some(&format!("export {var}=...")),
                );
            } else {
                match local_llm::list_models(&provider.base_url).await {
                    Some(models) if local_llm::has_model(&models, &provider.model) || models.is_empty() => report.line(
                        Status::Ok,
                        "llm",
                        &format!("{name}: {} at {}", provider.model, provider.base_url),
                        None,
                    ),
                    Some(_) if name == "ollama" => report.line(
                        Status::Warn,
                        "llm",
                        &format!("{name} is running but `{}` is not installed", provider.model),
                        Some(&format!("ollama pull {}", provider.model)),
                    ),
                    Some(_) => report.line(
                        Status::Warn,
                        "llm",
                        &format!("{name}: model `{}` not listed", provider.model),
                        Some("brok setup llm"),
                    ),
                    None => report.line(
                        Status::Warn,
                        "llm",
                        &format!("{name} not reachable at {}", provider.base_url),
                        Some("start Ollama / LM Studio, then `brok setup llm` (search works without it)"),
                    ),
                }
            }
        }
        None => report.line(Status::Fail, "llm", "llm.default is not a configured provider", Some("brok setup llm")),
    }

    match Registry::new(workspace.paths()).get(SYSTEM_AGENT) {
        Ok(Some(agent)) if agent.is_runnable() => {
            report.line(Status::Ok, "agent", &format!("{SYSTEM_AGENT} {}", agent.manifest.version), None)
        }
        Ok(Some(_)) => report.line(Status::Warn, "agent", "installed but its binary is missing", Some("brok init")),
        _ => report.line(
            Status::Warn,
            "agent",
            "brok-agent is not installed (needed by ask and skill run)",
            Some("install brok-agent next to brok, then `brok init`"),
        ),
    }

    if daemon::is_running(workspace).await {
        report.line(Status::Ok, "daemon", "running", None);
    } else {
        report.line(Status::Warn, "daemon", "not running", Some("brok run   (live indexing, ask, skill run)"));
    }

    let root = workspace.root();
    for agent in [Agent::Claude, Agent::Cursor, Agent::Vscode] {
        let detected = integrations::detect(root).contains(&agent);
        match integrations::mcp_configured(root, agent) {
            Some(true) => report.line(Status::Ok, "integration", &format!("{}: MCP + skills", agent.label()), None),
            _ if detected => report.line(
                Status::Warn,
                "integration",
                &format!("{} detected but not connected", agent.label()),
                Some(&format!("brok setup agents -a {}", format!("{agent:?}").to_lowercase())),
            ),
            _ => {}
        }
    }
}
