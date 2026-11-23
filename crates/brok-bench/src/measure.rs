//! Runs a command and measures what it costs the machine.
//!
//! `unsafe` is denied in this workspace, so this crate cannot call
//! `wait4`/`getrusage` itself. Two ways to get numbers instead:
//!
//! - **One-shot commands** run under `/usr/bin/time` (GNU on Linux, BSD on
//!   macOS) when it is available. The OS reports exact peak memory, CPU time
//!   and block I/O, even for a command that lives a few milliseconds. Peak
//!   memory is that of the largest process of the tree (`ru_maxrss`).
//! - **Daemons**, and one-shot commands where `time` is missing (Windows,
//!   minimal containers), are sampled with `sysinfo`. Consequences, stated
//!   so nobody over-reads a number:
//!
//! - peak memory is the highest *sampled* sum of resident memory of the
//!   tree (and, on Linux, the kernel's own high-water mark of the root
//!   process); a process that lives shorter than one interval may be missed;
//! - CPU time and disk bytes are the last values seen per process, summed;
//! - disk read/write bytes are 0 on platforms where the OS does not expose
//!   them per process.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// How often the process tree is sampled.
pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(25);

/// What one command cost.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measured {
    /// Wall-clock time.
    pub wall: Duration,
    /// CPU time of the tree, in milliseconds.
    pub cpu_ms: u64,
    /// Highest sampled resident memory of the tree, in bytes.
    pub peak_rss: u64,
    /// Resident memory of the tree at the last sample, in bytes: for a daemon
    /// this is the steady state, as opposed to the startup peak.
    pub final_rss: u64,
    /// Bytes read from disk.
    pub read_bytes: u64,
    /// Bytes written to disk.
    pub write_bytes: u64,
    /// Whether the command exited with status 0 (always false for a command
    /// that was stopped on purpose; see [`Measured::stopped`]).
    pub success: bool,
    /// The command was still running when the measuring window ended and was
    /// killed by the benchmark.
    pub stopped: bool,
}

impl Measured {
    /// Average CPU use over the wall time, in percent of one core.
    pub fn cpu_percent(&self) -> f64 {
        let wall = self.wall.as_secs_f64() * 1000.0;
        if wall <= 0.0 { 0.0 } else { self.cpu_ms as f64 / wall * 100.0 }
    }
}

/// The environment every benchmarked brok runs in: isolated from the user's
/// own configuration and with the counting `git` first on `PATH`.
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// Replaces the user configuration directory.
    pub config_dir: PathBuf,
    /// Embedding model cache (shared, so a pre-downloaded model is reused).
    pub models_dir: PathBuf,
    /// Directory holding the `git` shim, when it is supported.
    pub shim_dir: Option<PathBuf>,
    /// File the shim appends one line to for every `git` started.
    pub git_log: PathBuf,
}

impl Sandbox {
    /// Creates the sandbox under `base`. The shim needs a POSIX shell.
    pub fn create(base: &Path, models_dir: PathBuf) -> Result<Self> {
        let config_dir = base.join("config");
        std::fs::create_dir_all(&config_dir)?;
        let git_log = base.join("git-calls.log");
        std::fs::write(&git_log, b"")?;
        let shim_dir = install_git_shim(&base.join("shim"), &git_log)?;
        Ok(Self { config_dir, models_dir, shim_dir, git_log })
    }

    /// Applies the environment to a command.
    pub fn apply(&self, cmd: &mut Command) {
        cmd.env("BROK_CONFIG_DIR", &self.config_dir).env("BROK_MODELS_DIR", &self.models_dir);
        cmd.env("NO_COLOR", "1").env("RUST_LOG", "warn");
        if let Some(shim) = &self.shim_dir {
            let mut paths = vec![shim.clone()];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
    }

    /// `git` processes started so far, or `None` where the shim is unsupported.
    pub fn git_calls(&self) -> Option<u64> {
        self.shim_dir.as_ref()?;
        let text = std::fs::read_to_string(&self.git_log).ok()?;
        Some(text.lines().count() as u64)
    }
}

#[cfg(unix)]
fn install_git_shim(dir: &Path, log: &Path) -> Result<Option<PathBuf>> {
    use std::os::unix::fs::PermissionsExt;
    let Some(real) = find_in_path("git") else { return Ok(None) };
    std::fs::create_dir_all(dir)?;
    let script = format!("#!/bin/sh\nprintf '.\\n' >> '{}'\nexec '{}' \"$@\"\n", log.display(), real.display());
    let path = dir.join("git");
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(Some(dir.to_path_buf()))
}

#[cfg(not(unix))]
fn install_git_shim(_dir: &Path, _log: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(unix)]
fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Per-process values, kept at their highest so that a process that exits
/// between two samples still counts.
#[derive(Default, Clone, Copy)]
struct Seen {
    cpu_ms: u64,
    read: u64,
    write: u64,
}

struct Sampler {
    system: System,
    root: Pid,
    seen: HashMap<Pid, Seen>,
    peak_rss: u64,
    last_rss: u64,
}

impl Sampler {
    fn new(root: u32) -> Self {
        Self { system: System::new(), root: Pid::from_u32(root), seen: HashMap::new(), peak_rss: 0, last_rss: 0 }
    }

    fn sample(&mut self) {
        let kind = ProcessRefreshKind::nothing().with_memory().with_cpu().with_disk_usage();
        self.system.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let mut tree: HashSet<Pid> = HashSet::from([self.root]);
        // Parents are not ordered before children: repeat until nothing new joins.
        loop {
            let before = tree.len();
            for (pid, process) in self.system.processes() {
                // Linux lists every thread as a process that reports the whole
                // process's memory: counting them would multiply it.
                if process.thread_kind().is_some() {
                    continue;
                }
                if process.parent().is_some_and(|parent| tree.contains(&parent)) {
                    tree.insert(*pid);
                }
            }
            if tree.len() == before {
                break;
            }
        }
        let mut rss = 0;
        for pid in &tree {
            let Some(process) = self.system.process(*pid) else { continue };
            rss += process.memory();
            let entry = self.seen.entry(*pid).or_default();
            entry.cpu_ms = entry.cpu_ms.max(process.accumulated_cpu_time());
            let disk = process.disk_usage();
            entry.read = entry.read.max(disk.total_read_bytes);
            entry.write = entry.write.max(disk.total_written_bytes);
        }
        self.last_rss = rss;
        self.peak_rss = self.peak_rss.max(rss).max(kernel_high_water_mark(self.root.as_u32()).unwrap_or(0));
    }

    fn finish(&self, wall: Duration, success: bool, stopped: bool) -> Measured {
        Measured {
            wall,
            cpu_ms: self.seen.values().map(|s| s.cpu_ms).sum(),
            peak_rss: self.peak_rss,
            final_rss: self.last_rss,
            read_bytes: self.seen.values().map(|s| s.read).sum(),
            write_bytes: self.seen.values().map(|s| s.write).sum(),
            success,
            stopped,
        }
    }
}

/// Linux keeps the exact peak resident memory of a process (`VmHWM`).
fn kernel_high_water_mark(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

pub(crate) fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

/// Runs `cmd` until it exits and measures it, exactly when `/usr/bin/time`
/// can report for it, by sampling otherwise.
pub fn run_to_completion(cmd: &mut Command) -> Result<Measured> {
    match exact::flavor() {
        Some(flavor) => exact::run(cmd, flavor),
        None => sample_to_completion(cmd),
    }
}

fn sample_to_completion(cmd: &mut Command) -> Result<Measured> {
    quiet(cmd);
    let started = Instant::now();
    let mut child = cmd.spawn().with_context(|| format!("start {:?}", cmd.get_program()))?;
    let mut sampler = Sampler::new(child.id());
    loop {
        sampler.sample();
        if let Some(status) = child.try_wait()? {
            return Ok(sampler.finish(started.elapsed(), status.success(), false));
        }
        std::thread::sleep(SAMPLE_INTERVAL);
    }
}

/// Exact figures from the system `time` command (Unix).
mod exact {
    use super::{Measured, quiet};
    use anyhow::{Context, Result, bail};
    use std::process::Command;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    /// Which `time` is installed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Flavor {
        /// `/usr/bin/time -f ... -o file` (Linux).
        Gnu,
        /// `/usr/bin/time -l -o file` (macOS, BSD).
        Bsd,
    }

    pub fn flavor() -> Option<Flavor> {
        static FLAVOR: OnceLock<Option<Flavor>> = OnceLock::new();
        *FLAVOR.get_or_init(detect)
    }

    #[cfg(unix)]
    fn detect() -> Option<Flavor> {
        let dir = tempfile::tempdir().ok()?;
        let out = dir.path().join("probe");
        for flavor in [Flavor::Gnu, Flavor::Bsd] {
            let mut cmd = wrapper(flavor, &out);
            cmd.arg("true");
            quiet(&mut cmd);
            if cmd.status().is_ok_and(|s| s.success())
                && parse(flavor, &std::fs::read_to_string(&out).unwrap_or_default()).is_some()
            {
                return Some(flavor);
            }
        }
        None
    }

    #[cfg(not(unix))]
    fn detect() -> Option<Flavor> {
        None
    }

    fn wrapper(flavor: Flavor, out: &std::path::Path) -> Command {
        let mut cmd = Command::new("/usr/bin/time");
        match flavor {
            Flavor::Gnu => cmd.args(["-f", "%U %S %M %I %O", "-o"]),
            Flavor::Bsd => cmd.args(["-l", "-o"]),
        };
        cmd.arg(out);
        cmd
    }

    /// What `time` printed: (cpu ms, peak bytes, read bytes, written bytes).
    pub fn parse(flavor: Flavor, text: &str) -> Option<(u64, u64, u64, u64)> {
        match flavor {
            Flavor::Gnu => {
                // The last line is ours; an earlier "Command exited with non-zero status" line may precede it.
                let nums: Vec<f64> = text.lines().last()?.split_whitespace().filter_map(|w| w.parse().ok()).collect();
                let [user, sys, max_kib, blocks_in, blocks_out] = nums[..] else { return None };
                Some((
                    ((user + sys) * 1000.0).round() as u64,
                    max_kib as u64 * 1024,
                    blocks_in as u64 * 512,
                    blocks_out as u64 * 512,
                ))
            }
            Flavor::Bsd => {
                let value = |suffix: &str| -> Option<f64> {
                    text.lines().find(|l| l.trim_end().ends_with(suffix))?.split_whitespace().next()?.parse().ok()
                };
                let times = text.lines().find(|l| l.contains(" real"))?;
                let words: Vec<&str> = times.split_whitespace().collect();
                let at = |label: &str| -> Option<f64> {
                    words.iter().position(|w| *w == label).and_then(|i| words.get(i.checked_sub(1)?)?.parse().ok())
                };
                let cpu = (at("user")? + at("sys")?) * 1000.0;
                // 512-byte blocks on BSD as well.
                Some((
                    cpu.round() as u64,
                    value("maximum resident set size")? as u64,
                    value("block input operations")? as u64 * 512,
                    value("block output operations")? as u64 * 512,
                ))
            }
        }
    }

    pub fn run(cmd: &mut Command, flavor: Flavor) -> Result<Measured> {
        let dir = tempfile::tempdir().context("temporary file for `time`")?;
        let out = dir.path().join("time.txt");
        let mut wrapped = wrapper(flavor, &out);
        wrapped.arg(cmd.get_program()).args(cmd.get_args());
        if let Some(cwd) = cmd.get_current_dir() {
            wrapped.current_dir(cwd);
        }
        for (key, value) in cmd.get_envs() {
            match value {
                Some(value) => wrapped.env(key, value),
                None => wrapped.env_remove(key),
            };
        }
        quiet(&mut wrapped);
        let started = Instant::now();
        let status = wrapped.status().with_context(|| format!("start {:?}", cmd.get_program()))?;
        let wall: Duration = started.elapsed();
        let text = std::fs::read_to_string(&out).unwrap_or_default();
        let Some((cpu_ms, peak_rss, read_bytes, write_bytes)) = parse(flavor, &text) else {
            bail!("could not read the output of /usr/bin/time: {text:?}");
        };
        Ok(Measured {
            wall,
            cpu_ms,
            peak_rss,
            final_rss: 0,
            read_bytes,
            write_bytes,
            success: status.success(),
            stopped: false,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_gnu_time() {
            // user sys max-rss-KiB blocks-in blocks-out
            assert_eq!(parse(Flavor::Gnu, "0.50 0.25 2048 16 8\n"), Some((750, 2048 * 1024, 16 * 512, 8 * 512)));
            assert_eq!(
                parse(Flavor::Gnu, "Command exited with non-zero status 3\n0.01 0.00 1000 0 0\n"),
                Some((10, 1_024_000, 0, 0))
            );
            assert_eq!(parse(Flavor::Gnu, "garbage"), None);
        }

        #[test]
        fn parses_bsd_time() {
            let text = "        0.06 real         0.01 user         0.02 sys\n  5242880  maximum resident set size\n         0  page reclaims\n         4  block input operations\n         2  block output operations\n";
            assert_eq!(parse(Flavor::Bsd, text), Some((30, 5_242_880, 2048, 1024)));
        }

        #[cfg(unix)]
        #[test]
        fn short_commands_get_exact_numbers() {
            let Some(flavor) = flavor() else { return }; // no /usr/bin/time here
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo hi"]);
            let m = run(&mut cmd, flavor).unwrap();
            assert!(m.success);
            assert!(m.peak_rss > 0, "{m:?}");
        }

        #[cfg(unix)]
        #[test]
        fn failure_is_reported_and_env_is_passed() {
            let Some(flavor) = flavor() else { return };
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "test \"$BENCH_PROBE\" = yes && exit 3"]).env("BENCH_PROBE", "yes");
            let m = run(&mut cmd, flavor).unwrap();
            assert!(!m.success);
        }
    }
}

/// Starts a long-running command (a daemon), lets it settle for `warmup`,
/// then measures it for `window` and stops it. The warm-up is not counted
/// in the wall time, but memory peaks during it are kept.
///
/// `at_window_start` runs once the warm-up is over, to snapshot counters
/// that live outside the process (the `git` call log).
pub fn run_for(
    cmd: &mut Command,
    warmup: Duration,
    window: Duration,
    at_window_start: &mut dyn FnMut(),
) -> Result<(Measured, Option<std::process::ExitStatus>)> {
    let mut child = spawn_quiet(cmd)?;
    let mut sampler = Sampler::new(child.id());
    let settle = Instant::now();
    while settle.elapsed() < warmup {
        sampler.sample();
        if let Some(status) = child.try_wait()? {
            return Ok((sampler.finish(settle.elapsed(), false, false), Some(status)));
        }
        std::thread::sleep(SAMPLE_INTERVAL);
    }
    // Counters are cumulative: remember the warm-up values and subtract them.
    sampler.sample();
    at_window_start();
    let base_cpu: u64 = sampler.seen.values().map(|s| s.cpu_ms).sum();
    let base_read: u64 = sampler.seen.values().map(|s| s.read).sum();
    let base_write: u64 = sampler.seen.values().map(|s| s.write).sum();
    let started = Instant::now();
    let mut exited = None;
    while started.elapsed() < window {
        sampler.sample();
        if let Some(status) = child.try_wait()? {
            exited = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    sampler.sample();
    let wall = started.elapsed();
    if exited.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let mut measured = sampler.finish(wall, exited.is_none(), exited.is_none());
    measured.cpu_ms = measured.cpu_ms.saturating_sub(base_cpu);
    measured.read_bytes = measured.read_bytes.saturating_sub(base_read);
    measured.write_bytes = measured.write_bytes.saturating_sub(base_write);
    Ok((measured, exited))
}

/// A set of daemons measured together (several workspaces at once).
pub fn run_many_for(cmds: &mut [Command], warmup: Duration, window: Duration) -> Result<Measured> {
    let mut children: Vec<Child> = Vec::new();
    for cmd in cmds.iter_mut() {
        children.push(spawn_quiet(cmd)?);
    }
    let mut samplers: Vec<Sampler> = children.iter().map(|c| Sampler::new(c.id())).collect();
    let started = Instant::now();
    let mut base_cpu = 0;
    let mut based = false;
    let mut window_start = Instant::now();
    let mut peak_total = 0;
    while started.elapsed() < warmup + window {
        for s in &mut samplers {
            s.sample();
        }
        peak_total = peak_total.max(samplers.iter().map(|s| s.peak_rss).sum());
        if !based && started.elapsed() >= warmup {
            base_cpu = samplers.iter().flat_map(|s| s.seen.values()).map(|s| s.cpu_ms).sum();
            window_start = Instant::now();
            based = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let wall = window_start.elapsed();
    let total_cpu: u64 = samplers.iter().flat_map(|s| s.seen.values()).map(|s| s.cpu_ms).sum();
    let alive = children.iter_mut().all(|c| matches!(c.try_wait(), Ok(None)));
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok(Measured {
        wall,
        cpu_ms: total_cpu.saturating_sub(base_cpu),
        peak_rss: peak_total,
        final_rss: samplers.iter().map(|s| s.last_rss).sum(),
        read_bytes: samplers.iter().flat_map(|s| s.seen.values()).map(|s| s.read).sum(),
        write_bytes: samplers.iter().flat_map(|s| s.seen.values()).map(|s| s.write).sum(),
        success: alive,
        stopped: alive,
    })
}

fn spawn_quiet(cmd: &mut Command) -> Result<Child> {
    quiet(cmd);
    cmd.spawn().with_context(|| format!("start {:?}", cmd.get_program()))
}

/// Size of a directory tree in bytes (symlinks are not followed).
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    std::fs::read_dir(path).map_or(0, |entries| entries.flatten().map(|e| dir_size(&e.path())).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn measures_a_short_command() {
        let mut cmd = Command::new("sh");
        // Long enough for many samples even on a loaded machine: a process
        // that lives less than one interval can be missed (documented above).
        cmd.args(["-c", "sleep 0.8; echo done >/dev/null"]);
        let m = run_to_completion(&mut cmd).unwrap();
        assert!(m.success);
        assert!(m.wall >= Duration::from_millis(700), "{m:?}");
        assert!(m.peak_rss > 0, "{m:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_command_is_not_a_success() {
        let m = run_to_completion(Command::new("sh").args(["-c", "exit 3"])).unwrap();
        assert!(!m.success);
    }

    #[cfg(unix)]
    #[test]
    fn counts_git_processes_and_children() {
        let tmp = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::create(tmp.path(), tmp.path().join("models")).unwrap();
        if sandbox.shim_dir.is_none() {
            return; // no git on this machine
        }
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "git --version >/dev/null; git --version >/dev/null; sleep 0.1"]);
        sandbox.apply(&mut cmd);
        run_to_completion(&mut cmd).unwrap();
        assert_eq!(sandbox.git_calls(), Some(2));
    }

    /// Regression: threads were counted as processes, so a process with N
    /// threads reported N times its memory.
    #[cfg(target_os = "linux")]
    #[test]
    fn threads_are_not_counted_as_processes() {
        let script = "import threading, time\nblock = bytearray(64 * 1024 * 1024)\nfor i in range(0, len(block), 4096): block[i] = 1\nfor _ in range(16): threading.Thread(target=time.sleep, args=(2,)).start()\ntime.sleep(2)";
        let mut cmd = Command::new("python3");
        cmd.args(["-c", script]);
        let Ok(m) = sample_to_completion(&mut cmd) else { return }; // no python3 here
        if !m.success {
            return;
        }
        let mib = m.peak_rss / (1024 * 1024);
        // One copy of ~75 MiB, not sixteen.
        assert!((60..300).contains(&mib), "peak {mib} MiB");
    }

    #[cfg(unix)]
    #[test]
    fn a_daemon_is_stopped_after_the_window() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let (m, exited) =
            run_for(&mut cmd, Duration::from_millis(100), Duration::from_millis(400), &mut || {}).unwrap();
        assert!(exited.is_none());
        assert!(m.stopped);
        assert!(m.wall >= Duration::from_millis(350), "{m:?}");
    }

    #[test]
    fn dir_size_adds_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a")).unwrap();
        std::fs::write(tmp.path().join("a/x"), [0u8; 100]).unwrap();
        std::fs::write(tmp.path().join("y"), [0u8; 28]).unwrap();
        assert_eq!(dir_size(tmp.path()), 128);
        assert_eq!(dir_size(&tmp.path().join("missing")), 0);
    }
}
