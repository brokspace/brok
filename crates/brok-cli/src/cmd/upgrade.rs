//! `brok upgrade`: installs the latest release of `brok` and `brok-agent`,
//! shows what changed, and restarts the background brok of every workspace
//! that was running.
//!
//! Binaries come from the GitHub releases and are checked against their
//! `.sha256` files before anything is replaced.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use brok_core::Workspace;
use brok_shard::{SYSTEM_AGENT, find_executable};
use console::style;
use dialoguer::Confirm;
use semver::Version;
use serde::Deserialize;

use crate::{daemon, ui};

const BROK_REPO: &str = "brokspace/brok";
const AGENT_REPO: &str = "brokspace/brok-agent";

/// Arguments of `brok upgrade`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Only show whether a newer version exists and what changed.
    #[arg(long)]
    check: bool,
    /// Upgrade without asking.
    #[arg(short, long)]
    yes: bool,
    /// Install this brok version (e.g. `0.2.2`) instead of the latest.
    #[arg(long, value_name = "VERSION")]
    to: Option<Version>,
}

/// A GitHub release.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

impl Release {
    fn version(&self) -> Result<Version> {
        Version::parse(self.tag_name.trim_start_matches('v'))
            .with_context(|| format!("release tag `{}` is not a version", self.tag_name))
    }

    fn asset(&self, name: &str) -> Result<&Asset> {
        self.assets.iter().find(|asset| asset.name == name).with_context(|| {
            format!("release {} has no `{name}` (no prebuilt binary for this platform)", self.tag_name)
        })
    }
}

/// One program to replace.
struct Step {
    program: &'static str,
    path: PathBuf,
    from: Option<Version>,
    to: Version,
    release: Release,
}

pub async fn run(_cwd: &Path, args: Args) -> Result<()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let exe = std::env::current_exe().context("cannot locate the brok executable")?;

    let spinner = ui::spinner("Checking for a new version");
    let brok_release = fetch_release(BROK_REPO, args.to.as_ref(), Duration::from_secs(15)).await;
    let agent_release = fetch_release(AGENT_REPO, None, Duration::from_secs(15)).await;
    spinner.finish_and_clear();
    let brok_release = brok_release?;
    let target = brok_release.version()?;

    let mut steps = Vec::new();
    if target > current || args.to.as_ref().is_some_and(|to| *to != current) {
        steps.push(Step {
            program: "brok",
            path: exe.clone(),
            from: Some(current.clone()),
            to: target.clone(),
            release: brok_release,
        });
    }
    let agent_path = find_executable(SYSTEM_AGENT);
    let agent_version = match &agent_path {
        Some(path) => installed_version(path).await,
        None => None,
    };
    match agent_release {
        Ok(release) => {
            let latest = release.version()?;
            if agent_version.as_ref().is_none_or(|installed| latest > *installed) {
                let path = agent_path
                    .unwrap_or_else(|| exe.with_file_name(format!("{SYSTEM_AGENT}{}", std::env::consts::EXE_SUFFIX)));
                steps.push(Step { program: SYSTEM_AGENT, path, from: agent_version.clone(), to: latest, release });
            }
        }
        Err(e) => ui::warn(&format!("cannot check {SYSTEM_AGENT}: {e:#}")),
    }

    if steps.is_empty() {
        let agent = agent_version.map(|v| format!(" and {SYSTEM_AGENT} {v} are")).unwrap_or_else(|| " is".into());
        ui::success(&format!("brok {current}{agent} up to date"));
        return Ok(());
    }

    for step in &steps {
        let from = step.from.as_ref().map_or_else(|| "not installed".to_owned(), ToString::to_string);
        ui::step(&format!("{}: {from} → {}", step.program, style(&step.to).bold()));
    }
    if target > current {
        print_changes(&current, &target).await;
    }

    if installed_with_cargo(&exe) {
        println!();
        ui::step("brok was installed with Cargo; upgrade it the same way:");
        ui::hint("cargo install brok-cli brok-agent");
        return Ok(());
    }
    if args.check {
        println!();
        ui::hint("Install it with `brok upgrade`.");
        return Ok(());
    }
    if !args.yes {
        ensure!(ui::interactive(), "not a terminal: pass --yes to upgrade");
        if !Confirm::new().with_prompt("Upgrade now?").default(true).interact()? {
            ui::step("Cancelled");
            return Ok(());
        }
    }

    // Download and verify everything before touching what is installed.
    let spinner = ui::spinner("Downloading");
    let mut downloads = Vec::new();
    let mut failure = None;
    for step in &steps {
        match download(step).await {
            Ok(new) => downloads.push((step, new)),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    spinner.finish_and_clear();

    let running = running_workspaces(&exe);
    if failure.is_none() {
        for root in &running {
            if let Ok(workspace) = Workspace::discover(root)
                && let Err(e) = daemon::stop(&workspace).await
            {
                failure = Some(e.context(format!("stopping brok in {}", root.display())));
                break;
            }
        }
    }
    if let Some(e) = failure {
        for (_, new) in &downloads {
            let _ = std::fs::remove_file(new);
        }
        return Err(e);
    }
    for (step, new) in &downloads {
        replace(&step.path, new)?;
        ui::success(&format!("{} {}", step.program, step.to));
    }
    // The new brok starts the daemons, so they run the new code (and refresh
    // each workspace's copy of brok-agent).
    for root in &running {
        // Nothing of ours may reach the daemon: on Windows it would inherit
        // our output pipe and keep `brok upgrade | ...` waiting forever.
        let mut command = std::process::Command::new(&exe);
        command
            .arg("-C")
            .arg(root)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let status = daemon::without_inherited_std_handles(|| command.spawn()).and_then(|mut child| child.wait());
        match status {
            Ok(status) if status.success() => ui::success(&format!("Restarted brok in {}", root.display())),
            _ => ui::warn(&format!("could not restart brok in {}: run `brok start` there", root.display())),
        }
    }
    ui::hint(
        "Built-in skills may have changed: run `brok skill export` in your workspaces to refresh your coding agents.",
    );
    Ok(())
}

/// The newest brok release, or `None` when GitHub cannot be reached in time.
pub async fn latest_version(timeout: Duration) -> Option<Version> {
    fetch_release(BROK_REPO, None, timeout).await.ok()?.version().ok()
}

async fn fetch_release(repo: &str, version: Option<&Version>, timeout: Duration) -> Result<Release> {
    let url = match version {
        Some(version) => format!("https://api.github.com/repos/{repo}/releases/tags/v{version}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let mut request = http(timeout)?.get(&url).header("accept", "application/vnd.github+json");
    // Optional: a token lifts GitHub's limit of 60 anonymous requests an hour.
    if let Some(token) =
        ["GITHUB_TOKEN", "GH_TOKEN"].into_iter().find_map(|var| std::env::var(var).ok().filter(|t| !t.is_empty()))
    {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.with_context(|| format!("cannot reach GitHub ({url})"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        match version {
            Some(version) => bail!("{repo} has no release v{version}"),
            None => bail!("{repo} has no release yet"),
        }
    }
    Ok(response.error_for_status()?.json().await?)
}

fn http(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(timeout).user_agent(concat!("brok/", env!("CARGO_PKG_VERSION"))).build()?)
}

/// `brok-agent --version` prints `brok-agent 0.2.0`.
async fn installed_version(path: &Path) -> Option<Version> {
    let output = tokio::process::Command::new(path).arg("--version").output().await.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Version::parse(text.split_whitespace().last()?).ok()
}

/// Release asset of `program` for this platform, e.g. `brok-linux-x86_64`.
fn asset_name(program: &str) -> Result<String> {
    let os = match std::env::consts::OS {
        "linux" | "macos" | "windows" => std::env::consts::OS,
        other => bail!("no prebuilt binaries for {other}: use `cargo install brok-cli brok-agent`"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" | "aarch64" => std::env::consts::ARCH,
        other => bail!("no prebuilt binaries for {other}: use `cargo install brok-cli brok-agent`"),
    };
    Ok(format!("{program}-{os}-{arch}{}", std::env::consts::EXE_SUFFIX))
}

/// Downloads the new binary next to the one it replaces (same file system,
/// so the swap is a rename) and verifies it.
async fn download(step: &Step) -> Result<PathBuf> {
    let name = asset_name(step.program)?;
    let binary = step.release.asset(&name)?;
    let checksum = step.release.asset(&format!("{name}.sha256"))?;
    let expected = http(Duration::from_secs(30))?
        .get(&checksum.browser_download_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected = expected.split_whitespace().next().context("empty checksum file")?.to_owned();
    let new = with_suffix(&step.path, ".new");
    brok_shard::install::download_verified(&binary.browser_download_url, &expected, &new).await.with_context(|| {
        format!(
            "cannot write next to {} (run as a user who can, or reinstall with the install script)",
            step.path.display()
        )
    })?;
    Ok(new)
}

/// Puts `new` in place of `target`. A running program cannot be overwritten
/// on Windows, but it can be renamed: the old one moves aside first
/// (`brok.exe.old`, deleted by a later brok once nothing runs it).
fn replace(target: &Path, new: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755))?;
    }
    if cfg!(windows) && target.exists() {
        let mut old = with_suffix(target, ".old");
        if std::fs::remove_file(&old).is_err() && old.exists() {
            // An earlier old copy still runs (a daemon not restarted yet).
            old = with_suffix(target, &format!(".old-{}", std::process::id()));
        }
        std::fs::rename(target, &old).with_context(|| format!("cannot move {} aside", target.display()))?;
        if let Err(e) = std::fs::rename(new, target) {
            let _ = std::fs::rename(&old, target);
            return Err(e).with_context(|| format!("cannot replace {}", target.display()));
        }
        return Ok(());
    }
    std::fs::rename(new, target).with_context(|| format!("cannot replace {}", target.display()))
}

/// Deletes the copies an earlier upgrade moved aside (Windows only; a copy
/// still in use stays until the next run).
pub fn remove_leftovers() {
    if !cfg!(windows) {
        return;
    }
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with("brok.exe.old") || name.starts_with("brok-agent.exe.old")) && entry.path().is_file() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// `cargo install` puts binaries in `$CARGO_HOME/bin` (`~/.cargo/bin`).
fn installed_with_cargo(exe: &Path) -> bool {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let cargo_home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".cargo")));
    let canonical = |p: &Path| std::fs::canonicalize(p).ok();
    match (cargo_home, exe.parent()) {
        (Some(cargo_home), Some(dir)) => {
            canonical(dir).is_some_and(|dir| Some(dir) == canonical(&cargo_home.join("bin")))
        }
        _ => false,
    }
}

/// Workspaces whose background brok (`brok -C <root> run`) runs from `exe`.
fn running_workspaces(exe: &Path) -> Vec<PathBuf> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet),
    );
    let exe = std::fs::canonicalize(exe).ok();
    let own = sysinfo::Pid::from_u32(std::process::id());
    let mut roots: Vec<PathBuf> = system
        .processes()
        .iter()
        .filter(|(pid, process)| **pid != own && process.exe().and_then(|e| std::fs::canonicalize(e).ok()) == exe)
        .filter_map(|(_, process)| daemon_root(process.cmd(), process.cwd()))
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// The workspace a `brok ... run` command line serves: `-C <dir>`, or the
/// directory it was started in.
fn daemon_root(cmd: &[std::ffi::OsString], cwd: Option<&Path>) -> Option<PathBuf> {
    let args: Vec<_> = cmd.iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if !args.iter().any(|a| a == "run") {
        return None;
    }
    let dir = args.windows(2).find(|w| w[0] == "-C" || w[0] == "--dir").map(|w| PathBuf::from(&w[1]));
    dir.or_else(|| cwd.map(Path::to_path_buf))
}

/// Prints the CHANGELOG sections between the two versions.
async fn print_changes(from: &Version, to: &Version) {
    let url = format!("https://raw.githubusercontent.com/{BROK_REPO}/v{to}/CHANGELOG.md");
    let text = match http(Duration::from_secs(15)) {
        Ok(client) => match client.get(&url).send().await.and_then(reqwest::Response::error_for_status) {
            Ok(response) => response.text().await.ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let notes = text.map(|text| changelog_between(&text, from, to)).unwrap_or_default();
    println!();
    if notes.is_empty() {
        ui::hint(&format!("What's new: https://github.com/{BROK_REPO}/releases/tag/v{to}"));
        return;
    }
    println!("{}", style("What's new").bold());
    for line in notes.lines() {
        if let Some(version) = line.strip_prefix("## ") {
            println!("\n{}", style(version).bold().underlined());
        } else if let Some(section) = line.strip_prefix("### ") {
            println!("{}", style(section).cyan().bold());
        } else {
            println!("{line}");
        }
    }
}

/// The Keep a Changelog sections (`## [0.2.1] - 2026-10-30`) of versions
/// newer than `from`, up to and including `to`.
fn changelog_between(changelog: &str, from: &Version, to: &Version) -> String {
    let mut out = String::new();
    let mut keep = false;
    for line in changelog.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let version = heading.trim_start_matches('[').split([']', ' ']).next().and_then(|v| Version::parse(v).ok());
            keep = version.is_some_and(|v| v > *from && v <= *to);
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str = "# Changelog\n\n## [Unreleased]\n\n- next\n\n\
        ## [0.3.0] - 2026-12-01\n\n### Added\n\n- three\n\n\
        ## [0.2.2] - 2026-11-01\n\n### Fixed\n\n- two\n\n\
        ## [0.2.1] - 2026-10-30\n\n- one\n";

    #[test]
    fn changelog_sections_between_versions() {
        let v = |s: &str| Version::parse(s).unwrap();
        let notes = changelog_between(CHANGELOG, &v("0.2.1"), &v("0.3.0"));
        assert!(notes.starts_with("## [0.3.0] - 2026-12-01"), "{notes}");
        assert!(notes.contains("- three") && notes.contains("### Fixed\n\n- two"), "{notes}");
        assert!(!notes.contains("- one") && !notes.contains("- next"), "{notes}");
        assert_eq!(
            changelog_between(CHANGELOG, &v("0.2.1"), &v("0.2.2")),
            "## [0.2.2] - 2026-11-01\n\n### Fixed\n\n- two"
        );
        assert_eq!(changelog_between(CHANGELOG, &v("0.3.0"), &v("0.3.0")), "");
    }

    #[test]
    fn daemon_command_lines() {
        let cmd = |args: &[&str]| args.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        let cwd = Path::new("/elsewhere");
        assert_eq!(daemon_root(&cmd(&["brok", "-C", "/ws", "run"]), Some(cwd)), Some(PathBuf::from("/ws")));
        assert_eq!(daemon_root(&cmd(&["brok", "run"]), Some(cwd)), Some(PathBuf::from("/elsewhere")));
        assert_eq!(daemon_root(&cmd(&["brok", "-C", "/ws", "status"]), Some(cwd)), None);
    }

    #[test]
    fn replace_swaps_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(format!("brok{}", std::env::consts::EXE_SUFFIX));
        let new = with_suffix(&target, ".new");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        replace(&target, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(!new.exists());
    }

    #[test]
    fn asset_names_match_the_release_workflow() {
        let name = asset_name("brok").unwrap();
        assert!(name.starts_with("brok-") && name.ends_with(std::env::consts::EXE_SUFFIX), "{name}");
        assert!(["linux", "macos", "windows"].iter().any(|os| name.contains(os)), "{name}");
    }
}
