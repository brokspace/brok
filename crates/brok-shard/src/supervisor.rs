//! Runs shard processes for `brok run`.
//!
//! Each shard gets its own task that spawns the process in its sandbox,
//! forwards its output (prefixed with the shard name, and appended to
//! `.brokspace/logs/<name>.log`), and restarts it with exponential backoff
//! when it crashes. A shard that crashes 5 times within a minute is given
//! up on, so a broken shard never turns into a busy loop.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::registry::InstalledShard;
use crate::sandbox::SandboxEnv;

const MAX_CRASHES: usize = 5;
const CRASH_WINDOW: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Time a shard gets to exit on its own after `system:shutdown`.
pub const GRACE_PERIOD: Duration = Duration::from_secs(3);

/// Everything needed to launch one shard.
pub struct LaunchSpec {
    /// The shard.
    pub shard: InstalledShard,
    /// Its sandboxed environment (includes its session token).
    pub env: SandboxEnv,
    /// Working directory (the workspace root).
    pub cwd: PathBuf,
    /// Directory for log files.
    pub log_dir: PathBuf,
}

/// Supervises one shard until `shutdown` is cancelled.
pub async fn supervise(spec: LaunchSpec, shutdown: CancellationToken) {
    let name = spec.shard.manifest.name.clone();
    let Some(executable) = spec.shard.executable() else {
        tracing::debug!(shard = %name, "skill-only shard: nothing to run");
        return;
    };
    let mut crashes: VecDeque<Instant> = VecDeque::new();

    loop {
        let mut command = Command::new(&executable);
        command
            .args(&spec.shard.manifest.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        spec.env.apply(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                tracing::error!(shard = %name, error = %e, path = %executable.display(), "cannot start shard");
                return;
            }
        };
        tracing::info!(shard = %name, pid = child.id().unwrap_or_default(), "shard started");

        let log_path = spec.log_dir.join(format!("{name}.log"));
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(forward(name.clone(), stdout, log_path.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(forward(name.clone(), stderr, log_path));
        }

        let status: std::io::Result<ExitStatus> = tokio::select! {
            status = child.wait() => status,
            () = shutdown.cancelled() => {
                match tokio::time::timeout(GRACE_PERIOD, child.wait()).await {
                    Ok(_) => tracing::info!(shard = %name, "shard stopped"),
                    Err(_) => {
                        tracing::warn!(shard = %name, "shard did not exit in time; killing it");
                        let _ = child.kill().await;
                    }
                }
                return;
            }
        };

        match status {
            Ok(status) if status.success() => {
                tracing::info!(shard = %name, "shard exited");
                return;
            }
            Ok(status) => tracing::warn!(shard = %name, %status, "shard crashed"),
            Err(e) => tracing::warn!(shard = %name, error = %e, "lost track of shard"),
        }

        let now = Instant::now();
        crashes.push_back(now);
        while crashes.front().is_some_and(|t| now.duration_since(*t) > CRASH_WINDOW) {
            crashes.pop_front();
        }
        if crashes.len() >= MAX_CRASHES {
            tracing::error!(shard = %name, "shard keeps crashing; giving up (see .brokspace/logs/{name}.log)");
            return;
        }
        let backoff = Duration::from_secs(1 << (crashes.len() - 1)).min(MAX_BACKOFF);
        tracing::info!(shard = %name, seconds = backoff.as_secs(), "restarting shard");
        tokio::select! {
            () = tokio::time::sleep(backoff) => {}
            () = shutdown.cancelled() => return,
        }
    }
}

async fn forward(shard: String, stream: impl AsyncRead + Unpin, log_path: PathBuf) {
    let mut log = tokio::fs::OpenOptions::new().create(true).append(true).open(&log_path).await.ok();
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(file) = log.as_mut() {
            let _ = file.write_all(format!("{line}\n").as_bytes()).await;
        }
        tracing::info!(target: "shard", "[{shard}] {line}");
    }
}
