//! Talking to (and managing) the background `brok run` of a workspace.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use brok_core::Workspace;
use brok_sdk::{BrokClient, ConnectOptions, Inbox, Payload, transport};
use brok_shard::{Registry, SYSTEM_AGENT};
use serde::{Deserialize, Serialize};

use crate::ui;

/// Asks a running `brok run` to shut down gracefully.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DaemonStop {}
impl Payload for DaemonStop {
    const TOPIC: &'static str = "daemon:stop";
}

/// Asks a running `brok run` who is connected.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DaemonStatus {}
impl Payload for DaemonStatus {
    const TOPIC: &'static str = "daemon:status";
}

/// Reply to [`DaemonStatus`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonStatusReply {
    /// Process id of `brok run`.
    pub pid: u32,
    /// Connected identities (shards and services).
    pub identities: Vec<String>,
}
impl Payload for DaemonStatusReply {
    const TOPIC: &'static str = "daemon:status.result";
}

/// Whether `brok run` is serving this workspace.
pub async fn is_running(workspace: &Workspace) -> bool {
    transport::is_listening(&workspace.endpoint()).await
}

/// Connects to the running daemon with the operator token.
pub async fn connect(workspace: &Workspace) -> Result<(BrokClient, Inbox)> {
    if !is_running(workspace).await {
        bail!("brok is not running in this workspace. Start it with `brok start` (or `brok run` in a terminal)");
    }
    let token_file = workspace.paths().operator_token();
    let token = std::fs::read_to_string(&token_file)
        .with_context(|| format!("cannot read {} (is `brok run` running?)", token_file.display()))?;
    let options = ConnectOptions::new("brok-cli", env!("CARGO_PKG_VERSION"), token.trim());
    BrokClient::connect(&workspace.endpoint(), options).await.context("connecting to the brok event bus")
}

/// Connects, starting the daemon in the background first when needed.
pub async fn connect_or_start(workspace: &Workspace) -> Result<(BrokClient, Inbox)> {
    if !is_running(workspace).await {
        ui::step("Starting brok in the background (stop it with `brok stop`)");
        start(workspace).await?;
    }
    connect(workspace).await
}

/// The daemon's view of who is connected (`None` when it does not answer).
pub async fn status(workspace: &Workspace) -> Option<DaemonStatusReply> {
    let (client, _inbox) = connect(workspace).await.ok()?;
    client.call::<DaemonStatus, DaemonStatusReply>(&DaemonStatus {}, Duration::from_secs(3)).await.ok()
}

/// Log file of the background daemon.
pub fn log_file(workspace: &Workspace) -> PathBuf {
    workspace.paths().logs.join("brok.log")
}

/// Starts `brok run` detached from the terminal and waits until it is ready
/// (bus listening and, when installed, the agent connected).
pub async fn start(workspace: &Workspace) -> Result<u32> {
    if is_running(workspace).await {
        bail!("brok is already running for {}", workspace.root().display());
    }
    let log_path = log_file(workspace);
    std::fs::create_dir_all(&workspace.paths().logs)?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path)?;
    let exe = std::env::current_exe().context("cannot locate the brok executable")?;

    let mut command = std::process::Command::new(exe);
    command
        .arg("-C")
        .arg(workspace.root())
        .arg("run")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .env("NO_COLOR", "1");
    detach(&mut command);
    let child = without_inherited_std_handles(|| command.spawn()).context("cannot start `brok run`")?;
    let pid = child.id();
    drop(child); // the daemon outlives this process

    wait_ready(workspace, pid, Duration::from_secs(30)).await?;
    Ok(pid)
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0); // survives the terminal closing
}

#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

/// Runs `spawn` while this process' own stdin/stdout/stderr are not
/// inheritable. On Windows a child inherits every inheritable handle, so a
/// long-lived daemon would otherwise keep the caller's pipe open and
/// `brok ask | less` would never see end of file.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn without_inherited_std_handles<T>(spawn: impl FnOnce() -> T) -> T {
    use windows_sys::Win32::Foundation::{
        GetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};

    let mut restore = Vec::new();
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: only reads and flips the inherit flag of this process' own
        // standard handles; invalid or missing handles are skipped.
        unsafe {
            let handle = GetStdHandle(id);
            let mut flags = 0u32;
            if !handle.is_null() && handle != INVALID_HANDLE_VALUE && GetHandleInformation(handle, &mut flags) != 0 {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                restore.push((handle, flags & HANDLE_FLAG_INHERIT));
            }
        }
    }
    let result = spawn();
    for (handle, flag) in restore {
        // SAFETY: restores the flag captured above on the same handle.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, flag) };
    }
    result
}

#[cfg(unix)]
pub(crate) fn without_inherited_std_handles<T>(spawn: impl FnOnce() -> T) -> T {
    spawn() // Rust opens every descriptor with CLOEXEC; only the explicit stdio is passed.
}

async fn wait_ready(workspace: &Workspace, pid: u32, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let expect_agent = workspace.config().shard_enabled(SYSTEM_AGENT)
        && Registry::new(workspace.paths()).get(SYSTEM_AGENT).ok().flatten().is_some_and(|s| s.is_runnable());
    loop {
        if is_running(workspace).await {
            if !expect_agent {
                return Ok(());
            }
            if let Some(status) = status(workspace).await
                && status.identities.iter().any(|i| i == SYSTEM_AGENT)
            {
                return Ok(());
            }
        }
        if Instant::now() > deadline {
            bail!(
                "brok (pid {pid}) did not become ready in {}s; see {}",
                timeout.as_secs(),
                log_file(workspace).display()
            );
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Asks the daemon to stop and waits until it has.
/// Returns once the process has exited, not just closed the bus: until then
/// Windows keeps `brok.exe` locked, and replacing it (an upgrade) fails.
pub async fn stop(workspace: &Workspace) -> Result<bool> {
    if !is_running(workspace).await {
        return Ok(false);
    }
    let pid = std::fs::read_to_string(workspace.paths().pid_file()).ok().and_then(|s| s.trim().parse::<u32>().ok());
    let (client, _inbox) = connect(workspace).await?;
    client.emit(&DaemonStop {}).await?;
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(15);
    while is_running(workspace).await || pid.is_some_and(process_alive) {
        if Instant::now() > deadline {
            bail!("brok did not stop within 15s; see {}", log_file(workspace).display());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(true)
}

/// Whether a process with this id is still running (a zombie counts as gone).
fn process_alive(pid: u32) -> bool {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).is_some_and(|p| p.status() != sysinfo::ProcessStatus::Zombie)
}

#[cfg(test)]
mod tests {
    use super::process_alive;

    #[test]
    fn process_alive_tracks_exit() {
        assert!(process_alive(std::process::id()));
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_alive(pid));
    }
}
