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
    /// Services still starting, which cannot receive requests yet. A daemon
    /// from before this field sends none.
    #[serde(default)]
    pub starting: Vec<String>,
    /// Whether a shard is subscribed to `agent:ask`. A daemon from before
    /// this field does not say.
    #[serde(default)]
    pub answers_asks: Option<bool>,
}
impl Payload for DaemonStatusReply {
    const TOPIC: &'static str = "daemon:status.result";
}

/// Bus identity of the index service.
pub const INDEX_SERVICE: &str = "brok-index";

/// How long to wait for a starting daemon.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Whether `brok run` is serving this workspace.
pub async fn is_running(workspace: &Workspace) -> bool {
    transport::is_listening(&workspace.endpoint()).await
}

/// Connects to the running daemon once its index service can answer: a
/// request published before the service subscribed would reach nobody.
pub async fn connect(workspace: &Workspace) -> Result<(BrokClient, Inbox)> {
    let connection = connect_now(workspace).await?;
    wait_started(&connection.0, &[INDEX_SERVICE], READY_TIMEOUT).await;
    Ok(connection)
}

/// Connects without waiting for the daemon to finish starting.
async fn connect_now(workspace: &Workspace) -> Result<(BrokClient, Inbox)> {
    if !is_running(workspace).await {
        bail!("brok is not running in this workspace. Start it with `brok start` (or `brok run` in a terminal)");
    }
    let token_file = workspace.paths().operator_token();
    let token = std::fs::read_to_string(&token_file)
        .with_context(|| format!("cannot read {} (is `brok run` running?)", token_file.display()))?;
    let options = ConnectOptions::new("brok-cli", env!("CARGO_PKG_VERSION"), token.trim());
    BrokClient::connect(&workspace.endpoint(), options).await.context("connecting to the brok event bus")
}

/// Waits until none of `services` is still starting. Gives up quietly after
/// `timeout`: the request that follows reports what is wrong.
async fn wait_started(client: &BrokClient, services: &[&str], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // No answer means the control service is not listening yet.
        if let Ok(status) =
            client.call::<DaemonStatus, DaemonStatusReply>(&DaemonStatus {}, Duration::from_millis(500)).await
            && !status.starting.iter().any(|s| services.contains(&s.as_str()))
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Connects, starting the daemon in the background first when needed, and
/// waits for the agent too: callers of this ask it something.
pub async fn connect_or_start(workspace: &Workspace) -> Result<(BrokClient, Inbox)> {
    if !is_running(workspace).await {
        ui::note("Starting brok in the background (stop it with `brok stop`)");
        start(workspace).await?;
    }
    let connection = connect(workspace).await?;
    wait_started(&connection.0, &[SYSTEM_AGENT], READY_TIMEOUT).await;
    // Without a subscriber the question would go nowhere and the caller would
    // wait for minutes.
    if let Ok(status) =
        connection.0.call::<DaemonStatus, DaemonStatusReply>(&DaemonStatus {}, Duration::from_secs(3)).await
        && status.answers_asks == Some(false)
    {
        bail!("{}", nobody_answers(AgentState::of(workspace)));
    }
    Ok(connection)
}

/// Why `brok-agent` may be missing from the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentState {
    Disabled,
    NotInstalled,
    NoBinary,
    NotConnected,
}

impl AgentState {
    fn of(workspace: &Workspace) -> Self {
        if !workspace.config().shard_enabled(SYSTEM_AGENT) {
            return Self::Disabled;
        }
        match Registry::new(workspace.paths()).get(SYSTEM_AGENT) {
            Ok(Some(agent)) if agent.is_runnable() => Self::NotConnected,
            Ok(Some(_)) => Self::NoBinary,
            _ => Self::NotInstalled,
        }
    }
}

/// What happened and the one thing to do next.
fn nobody_answers(agent: AgentState) -> String {
    let why = match agent {
        AgentState::Disabled => {
            "brok-agent is turned off in brok.yml. Run `brok config set shards.brok-agent.enabled true`, then `brok restart`"
        }
        AgentState::NotInstalled => {
            "brok-agent is not installed. Put it next to brok (the install script does), run `brok init`, then `brok restart`"
        }
        AgentState::NoBinary => "the brok-agent program is missing. Run `brok init`, then `brok restart`",
        AgentState::NotConnected => {
            "brok-agent is installed but not connected. See why with `brok logs`, then run `brok restart`"
        }
    };
    format!("nothing can answer: {why}")
}

/// The daemon's view of who is connected (`None` when it does not answer).
pub async fn status(workspace: &Workspace) -> Option<DaemonStatusReply> {
    let (client, _inbox) = connect_now(workspace).await.ok()?;
    client.call::<DaemonStatus, DaemonStatusReply>(&DaemonStatus {}, Duration::from_secs(3)).await.ok()
}

/// Log file of the background daemon.
pub fn log_file(workspace: &Workspace) -> PathBuf {
    workspace.paths().logs.join("brok.log")
}

/// Starts `brok run` detached from the terminal and waits until it is ready
/// (index service listening and, when installed, the agent too).
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

    wait_ready(workspace, pid, READY_TIMEOUT).await?;
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
        // An agent that has not connected stops counting as starting after a
        // grace period: brok is then ready, without it.
        if is_running(workspace).await
            && let Some(status) = status(workspace).await
            && status.starting.is_empty()
        {
            if expect_agent && !status.identities.iter().any(|i| i == SYSTEM_AGENT) {
                ui::warn(&nobody_answers(AgentState::NotConnected));
            }
            return Ok(());
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
    let (client, _inbox) = connect_now(workspace).await?;
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use brok_bus::{Bus, Grant};
    use brok_core::Workspace;
    use brok_sdk::payload::{SearchRequest, SearchResults};
    use brok_sdk::{BrokClient, ConnectOptions, TopicPattern};
    use tokio_util::sync::CancellationToken;

    use super::{DaemonStatus, DaemonStatusReply, INDEX_SERVICE, connect, process_alive};

    async fn service(bus: &Bus, name: &str, topics: &str) -> (BrokClient, brok_sdk::Inbox) {
        let pattern = TopicPattern::new(topics).unwrap();
        let namespace = TopicPattern::new(format!("{}:*", topics.split(':').next().unwrap())).unwrap();
        let grant = Grant::service(name, vec![namespace], vec![pattern]);
        let token = bus.authenticator().issue(grant);
        let options = ConnectOptions::new(name, "0.0.0", token.expose());
        BrokClient::connect(bus.endpoint(), options).await.unwrap()
    }

    #[test]
    fn a_missing_agent_is_explained_with_one_next_step() {
        use super::{AgentState, nobody_answers};
        for state in [AgentState::Disabled, AgentState::NotInstalled, AgentState::NoBinary, AgentState::NotConnected] {
            let text = nobody_answers(state);
            assert!(text.starts_with("nothing can answer: "), "{text}");
            assert!(text.contains("`brok restart`"), "{state:?}: {text}");
        }
    }

    #[tokio::test]
    async fn a_search_waits_for_a_slow_index_service() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".brokspace/run")).unwrap();
        std::fs::write(dir.path().join(".brokspace/brok.yml"), "version: 1\n").unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        let shutdown = CancellationToken::new();
        let bus = Bus::start(workspace.endpoint(), shutdown.clone()).unwrap();
        let context = brok_sdk::frame::ShardContext {
            workspace_root: dir.path().to_path_buf(),
            settings: serde_json::Value::Null,
            llm: serde_json::Value::Null,
            members: Vec::new(),
        };
        let operator = bus.authenticator().issue(Grant::operator(context));
        std::fs::write(workspace.paths().operator_token(), operator.expose()).unwrap();

        let ready = Arc::new(AtomicBool::new(false));
        let (control, mut requests) = service(&bus, "brok-daemon", "daemon:status").await;
        control.subscribe([TopicPattern::new("daemon:status").unwrap()]).await.unwrap();
        let answering = Arc::clone(&ready);
        tokio::spawn(async move {
            while let Some(event) = requests.recv().await {
                if event.is::<DaemonStatus>() {
                    let starting =
                        if answering.load(Ordering::SeqCst) { vec![] } else { vec![INDEX_SERVICE.to_owned()] };
                    let reply = DaemonStatusReply { starting, ..Default::default() };
                    control.reply(&event, &reply).await.unwrap();
                }
            }
        });
        // The index service joins late, as when a model takes a while to load.
        let index_bus = bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let (client, mut inbox) = service(&index_bus, INDEX_SERVICE, "index:search").await;
            client.subscribe([TopicPattern::new("index:search").unwrap()]).await.unwrap();
            while !index_bus.stats().await.subscribed.iter().any(|s| s == INDEX_SERVICE) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            ready.store(true, Ordering::SeqCst);
            while let Some(event) = inbox.recv().await {
                client.reply(&event, &SearchResults::default()).await.unwrap();
            }
        });

        let (client, _inbox) = connect(&workspace).await.unwrap();
        let request = SearchRequest::new("anything");
        client.call::<SearchRequest, SearchResults>(&request, Duration::from_secs(5)).await.unwrap();
        shutdown.cancel();
    }

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
