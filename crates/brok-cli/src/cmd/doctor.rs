//! `brok doctor`: checks everything Brokspace needs and says how to fix it.

use std::path::Path;

use anyhow::{Result, bail};
use brok_core::Workspace;
use brok_core::config::{Device, EmbeddingsConfig};
use brok_index::{Access, Index, ModelFiles};
use brok_shard::{Registry, SYSTEM_AGENT};
use console::style;
use serde::Serialize;

use crate::integrations::{self, Agent};
use crate::{daemon, local_llm};

/// Arguments of `brok doctor`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Print the checks as JSON (`[{"status", "name", "detail", "fix"}]`) and exit 0.
    #[arg(long)]
    json: bool,
}

/// Outcome of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Works.
    Ok,
    /// Works, but something would unlock more.
    Warn,
    /// Needs attention.
    Fail,
}

/// One check of `brok doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// How it went.
    pub status: Status,
    /// What was checked (`llm`, `index`, ...).
    pub name: String,
    /// What was found.
    pub detail: String,
    /// The command or step that fixes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Default)]
struct Report {
    checks: Vec<Check>,
}

impl Report {
    fn line(&mut self, status: Status, name: &str, detail: &str, fix: Option<&str>) {
        self.checks.push(Check {
            status,
            name: name.to_owned(),
            detail: detail.to_owned(),
            fix: fix.map(str::to_owned),
        });
    }
}

pub async fn run(cwd: &Path, args: Args) -> Result<()> {
    let checks = checks(cwd).await;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
        return Ok(());
    }
    for check in &checks {
        let mark = match check.status {
            Status::Ok => style("✓").green().bold(),
            Status::Warn => style("!").yellow().bold(),
            Status::Fail => style("✗").red().bold(),
        };
        println!("{mark} {:<14} {}", style(&check.name).bold(), check.detail);
        if let Some(fix) = &check.fix {
            println!("  {:<14} {}", "", style(format!("fix: {fix}")).dim());
        }
    }
    let count = |status| checks.iter().filter(|c| c.status == status).count();
    println!();
    match (count(Status::Fail), count(Status::Warn)) {
        (0, 0) => println!("{}", style("Everything looks good.").green()),
        (0, w) => println!("{}", style(format!("{w} suggestion(s); Brokspace works, these unlock more.")).yellow()),
        (f, _) => bail!("{f} problem(s) need attention"),
    }
    Ok(())
}

/// Runs every check (installation, workspace, index, model, LLM, agent,
/// daemon, integrations, linked workspaces).
pub async fn checks(cwd: &Path) -> Vec<Check> {
    let mut report = Report::default();
    let features = brok_index::compiled_features();
    let build = if features.is_empty() { "lean build: text search only".to_owned() } else { features.join(", ") };
    report.line(Status::Ok, "brok", &format!("{} ({build})", env!("CARGO_PKG_VERSION")), None);
    // Offline or rate limited: say nothing rather than slow the check down.
    if let Some(latest) = super::upgrade::latest_version(std::time::Duration::from_secs(3)).await
        && semver::Version::parse(env!("CARGO_PKG_VERSION")).is_ok_and(|current| latest > current)
    {
        report.line(Status::Warn, "update", &format!("brok {latest} is available"), Some("brok upgrade"));
    }

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
    report.checks
}

async fn check_workspace(workspace: &Workspace, report: &mut Report) {
    let config = workspace.config();
    report.line(Status::Ok, "workspace", &workspace.root().display().to_string(), None);

    // Index files sit ~110 characters below the workspace root.
    let deepest = workspace.root().as_os_str().len() + 110;
    if cfg!(windows) && deepest > 255 && !windows_long_paths_enabled() {
        report.line(
            Status::Warn,
            "paths",
            "the workspace path is long and Windows long paths are disabled",
            Some(r"enable them (admin PowerShell: Set-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem LongPathsEnabled 1) or use a shorter path"),
        );
    }

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

    if let Some((detail, fix)) = costly_batch(&config.embeddings, GPU_BUILD) {
        report.line(Status::Warn, "batch size", &detail, Some(fix));
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
        Ok(Some(agent)) if agent.is_runnable() => match brok_shard::install::check_compatible(&agent.manifest) {
            Ok(()) => report.line(Status::Ok, "agent", &format!("{SYSTEM_AGENT} {}", agent.manifest.version), None),
            Err(e) => {
                report.line(Status::Fail, "agent", &format!("{e:#}"), Some("brok upgrade   (then `brok restart`)"))
            }
        },
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
        report.line(
            Status::Warn,
            "daemon",
            "not running",
            Some("brok start   (live indexing; ask, chat and skill run also start it)"),
        );
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

    if let Some((detail, fix)) = tracked_private_files(workspace.root()) {
        report.line(Status::Fail, "private files", &detail, Some(&fix));
    }

    for provider in brok_core::provider::ProviderRegistry::builtin().list() {
        let capabilities: Vec<&str> = provider.capabilities.iter().map(|c| c.as_str()).collect();
        let detail = if capabilities.is_empty() {
            format!("{} (contract v{})", provider.id, provider.contract_version)
        } else {
            format!("{} (contract v{}; {})", provider.id, provider.contract_version, capabilities.join(", "))
        };
        report.line(Status::Ok, "source control", &detail, None);
    }

    for member in brok_core::members::Member::all(workspace) {
        match member.state() {
            brok_core::members::MemberState::Missing => report.line(
                Status::Warn,
                "member",
                &format!("{}: {} does not exist on this machine", member.id, member.root.display()),
                Some(&format!("brok workspace remove {}   (or restore the directory)", member.id)),
            ),
            brok_core::members::MemberState::NotCloned => report.line(
                Status::Warn,
                "member",
                &format!("{}: not cloned yet, so `--scope workspace` skips it", member.id),
                Some(&format!("brok workspace sync {}", member.id)),
            ),
            _ if member.is_git() => {
                report.line(Status::Ok, "member", &format!("{}: searchable (cloned by brok)", member.id), None)
            }
            _ if member.is_workspace() => {
                report.line(Status::Ok, "member", &format!("{}: searchable (own brokspace)", member.id), None);
            }
            _ => report.line(
                Status::Warn,
                "member",
                &format!("{}: no Brokspace of its own, so `--scope workspace` skips it", member.id),
                Some(&format!("brok -C {} init", member.root.display())),
            ),
        }
    }

    if let Ok(links) = super::link::statuses(workspace).await {
        for link in links {
            match link.error {
                None => report.line(
                    Status::Ok,
                    "link",
                    &format!("{}: {} files, {} chunks", link.name, link.files, link.chunks),
                    None,
                ),
                Some(error) => report.line(
                    Status::Warn,
                    "link",
                    &format!("{}: {error}", link.name),
                    Some(&format!("brok link sync {}", link.name)),
                ),
            }
        }
    }
}

/// Whether this build can run the model on a GPU, where large batches pay off.
const GPU_BUILD: bool = cfg!(any(feature = "cuda", feature = "coreml", feature = "directml"));

/// Largest batch worth no warning on a CPU: about 1 GB at peak.
const CPU_BATCH_LIMIT: usize = 4;

/// What to say about a batch size that costs a CPU gigabytes for nothing
/// (`brok.yml` files written by brok 0.9 and earlier say 16).
fn costly_batch(embeddings: &EmbeddingsConfig, gpu_build: bool) -> Option<(String, &'static str)> {
    let on_cpu = embeddings.device == Device::Cpu || (embeddings.device == Device::Auto && !gpu_build);
    (embeddings.enabled && on_cpu && embeddings.batch_size > CPU_BATCH_LIMIT).then(|| {
        (
            format!(
                "batch_size {} can take gigabytes while indexing on a CPU, without being faster",
                embeddings.batch_size
            ),
            "brok config set embeddings.batch_size 1   (then `brok restart`)",
        )
    })
}

/// Private files git already tracks (an ignore rule does not untrack them),
/// and what to do about it. Folders the user opted in with `!` are left out.
fn tracked_private_files(root: &Path) -> Option<(String, String)> {
    let rules = std::fs::read_to_string(root.join(".brokspace/.gitignore")).unwrap_or_default();
    let opted_in = |dir: &str| {
        rules.lines().any(|l| l.trim().strip_prefix('!').is_some_and(|r| r.trim_matches('/').starts_with(dir)))
    };
    let paths: Vec<String> = brok_core::init::PRIVATE_DIRS
        .iter()
        .filter(|dir| !opted_in(dir))
        .map(|dir| format!(".brokspace/{dir}"))
        .collect();
    let mut args = vec!["ls-files", "--"];
    args.extend(paths.iter().map(String::as_str));
    // Not a git repository: nothing can be committed.
    let tracked = brok_core::git::run(root, &args).ok()?;
    let files: Vec<&str> = tracked.lines().collect();
    if files.is_empty() {
        return None;
    }
    let mut folders: Vec<String> =
        files.iter().filter_map(|f| f.split('/').nth(1)).map(|dir| format!(".brokspace/{dir}")).collect();
    folders.sort();
    folders.dedup();
    Some((
        format!("{} private files are tracked by git, for example {}", files.len(), files[0]),
        format!(
            "git rm -r --cached {}   (then commit; they stay in the history, so rotate any secret and see docs/troubleshooting.md)",
            folders.join(" ")
        ),
    ))
}

/// Whether Windows allows paths longer than 260 characters.
fn windows_long_paths_enabled() -> bool {
    std::process::Command::new("reg")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "/v", "LongPathsEnabled"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("0x1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_private_files_git_already_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| brok_core::git::run(dir.path(), args).unwrap();
        git(&["init", "-q"]);
        for file in [".brokspace/memory/a.md", ".brokspace/metrics/m.jsonl", ".brokspace/brok.yml"] {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        assert!(tracked_private_files(dir.path()).is_none(), "untracked files are not a problem yet");
        git(&["add", "-f", ".brokspace"]);
        let (detail, fix) = tracked_private_files(dir.path()).unwrap();
        assert!(detail.starts_with("2 private files"), "{detail}");
        assert!(fix.starts_with("git rm -r --cached .brokspace/memory .brokspace/metrics "), "{fix}");
        std::fs::write(dir.path().join(".brokspace/.gitignore"), "!memory/\n!metrics/\n").unwrap();
        assert!(tracked_private_files(dir.path()).is_none(), "shared on purpose");
        assert!(tracked_private_files(&std::env::temp_dir().join("not-a-repository")).is_none());
    }

    #[test]
    fn warns_about_large_batches_on_a_cpu_only() {
        let with = |batch_size, device| EmbeddingsConfig { batch_size, device, ..EmbeddingsConfig::default() };
        let (detail, fix) = costly_batch(&with(16, Device::Auto), false).unwrap();
        assert!(detail.contains("batch_size 16"), "{detail}");
        assert!(fix.starts_with("brok config set embeddings.batch_size 1"));
        assert!(costly_batch(&with(16, Device::Cpu), true).is_some());
        assert!(costly_batch(&EmbeddingsConfig::default(), false).is_none());
        assert!(costly_batch(&with(CPU_BATCH_LIMIT, Device::Auto), false).is_none());
        assert!(costly_batch(&with(16, Device::Auto), true).is_none(), "a GPU build may use it");
        assert!(costly_batch(&with(16, Device::Cuda), false).is_none());
        let off = EmbeddingsConfig { enabled: false, ..with(16, Device::Auto) };
        assert!(costly_batch(&off, false).is_none());
    }
}
