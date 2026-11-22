//! The benchmark scenarios of brokspace/brok#71.
//!
//! Every scenario drives the real `brok` binary against a synthetic
//! workspace in a temporary directory, with an isolated user configuration.
//! Semantic search needs the embedding model (a download), so the default
//! mode is text-only and fully offline; `--semantic` opts in.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::fixture::{self, Fixture, Size};
use crate::measure::{self, Measured, Sandbox, dir_size};
use crate::report::{Environment, FixtureInfo, Report, SCHEMA_VERSION, Scenario, Status};

/// Scenario names, in execution order.
pub const ALL: [&str; 9] = [
    "init",
    "index-initial",
    "index-noop",
    "index-incremental",
    "search",
    "idle-daemon",
    "multi-repo",
    "multi-workspace-idle",
    "llm-answer",
];

/// What to run.
#[derive(Debug, Clone)]
pub struct Options {
    /// The `brok` binary to benchmark.
    pub brok: PathBuf,
    /// Workspace size.
    pub size: Size,
    /// Include semantic search (downloads the model if it is not cached).
    pub semantic: bool,
    /// Length of the idle measurement window, in seconds.
    pub idle_secs: u64,
    /// Let `brok init` install the default `brok-agent` shard (network).
    pub with_agent: bool,
    /// Run only these scenarios (empty: all).
    pub only: Vec<String>,
    /// Shared model cache.
    pub models_dir: PathBuf,
}

struct Ctx<'a> {
    opts: &'a Options,
    sandbox: Sandbox,
}

impl Ctx<'_> {
    fn brok(&self, ws: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.opts.brok);
        cmd.arg("-C").arg(ws).args(args);
        self.sandbox.apply(&mut cmd);
        cmd
    }

    fn setup(&self, ws: &Path) -> Result<()> {
        let mut args = vec!["init", "-y", "--no-llm", "--no-index", "--no-integrations", "--skip-model"];
        if !self.opts.with_agent {
            args.push("--no-agent");
        }
        self.run_ok(ws, &args)?;
        if !self.opts.semantic {
            self.run_ok(ws, &["config", "set", "embeddings.enabled", "false"])?;
        }
        Ok(())
    }

    fn run_ok(&self, ws: &Path, args: &[&str]) -> Result<()> {
        let status = self.brok(ws, args).output().with_context(|| format!("run brok {}", args.join(" ")))?;
        if !status.status.success() {
            bail!("`brok {}` failed: {}", args.join(" "), String::from_utf8_lossy(&status.stderr).trim());
        }
        Ok(())
    }
}

fn wanted(opts: &Options, name: &str) -> bool {
    opts.only.is_empty() || opts.only.iter().any(|n| n == name)
}

fn fill(s: &mut Scenario, m: &Measured) {
    s.wall_ms = Some(m.wall.as_millis() as u64);
    s.cpu_ms = Some(m.cpu_ms);
    s.cpu_percent = Some((m.cpu_percent() * 10.0).round() / 10.0);
    s.peak_rss_bytes = Some(m.peak_rss);
    s.io_read_bytes = Some(m.read_bytes);
    s.io_write_bytes = Some(m.write_bytes);
}

fn disk(s: &mut Scenario, ws: &Path) {
    s.index_bytes = Some(dir_size(&ws.join(".brokspace").join("databases")));
    s.workspace_state_bytes = Some(dir_size(&ws.join(".brokspace")));
}

fn one_shot(ctx: &Ctx<'_>, name: &str, ws: &Path, args: &[&str]) -> Result<Scenario> {
    let before = ctx.sandbox.git_calls();
    let m = measure::run_to_completion(&mut ctx.brok(ws, args))?;
    let mut s = Scenario::new(name, if m.success { Status::Passed } else { Status::Failed });
    if !m.success {
        s.reason = Some(format!("`brok {}` exited with an error", args.join(" ")));
    }
    fill(&mut s, &m);
    disk(&mut s, ws);
    s.git_subprocesses = ctx.sandbox.git_calls().zip(before).map(|(a, b)| a - b);
    Ok(s)
}

/// Runs the scenarios and returns the report.
pub fn run(opts: &Options) -> Result<Report> {
    let version = Command::new(&opts.brok)
        .arg("--version")
        .output()
        .with_context(|| format!("run {} --version", opts.brok.display()))?;
    let brok_version = String::from_utf8_lossy(&version.stdout).trim().to_owned();

    let base = tempfile::tempdir().context("create the benchmark directory")?;
    std::fs::create_dir_all(&opts.models_dir)?;
    let sandbox = Sandbox::create(base.path(), opts.models_dir.clone())?;
    let ctx = Ctx { opts, sandbox };

    let ws = base.path().join("ws");
    let fixture = fixture::generate(&ws, opts.size, 0)?;
    let mut scenarios: Vec<Scenario> = Vec::new();

    // 1. Fresh installation and initialization.
    let init_ok;
    {
        let mut args = vec!["init", "-y", "--no-llm", "--no-index", "--no-integrations", "--skip-model"];
        if !opts.with_agent {
            args.push("--no-agent");
        }
        let s = one_shot(&ctx, "init", &ws, &args)?;
        init_ok = s.status == Status::Passed;
        if !opts.semantic && init_ok {
            ctx.run_ok(&ws, &["config", "set", "embeddings.enabled", "false"])?;
        }
        if opts.semantic && init_ok {
            // Not measured: a network download, and shared between runs.
            ctx.run_ok(&ws, &["model", "download"])
                .context("download the embedding model (use --models-dir to reuse one)")?;
        }
        if wanted(opts, "init") {
            scenarios.push(s);
        }
    }
    // Picking a later scenario alone still needs an index to measure.
    let index_dependents = ["index-noop", "index-incremental", "search", "idle-daemon"];
    if init_ok && !wanted(opts, "index-initial") && index_dependents.iter().any(|n| wanted(opts, n)) {
        ctx.run_ok(&ws, &["index"])?;
    }
    let skip_rest = |name: &str| Scenario::skipped(name, "init failed, so nothing else could run");

    for name in ["index-initial", "index-noop", "index-incremental", "search", "idle-daemon"] {
        if !wanted(opts, name) {
            continue;
        }
        if !init_ok {
            scenarios.push(skip_rest(name));
            continue;
        }
        let s = match name {
            "index-initial" => one_shot(&ctx, name, &ws, &["index"])?,
            "index-noop" => one_shot(&ctx, name, &ws, &["index"])?,
            "index-incremental" => {
                fixture::touch_one_file(&ws)?;
                one_shot(&ctx, name, &ws, &["index"])?
            }
            "search" => search(&ctx, &ws, &fixture)?,
            _ => idle_daemon(&ctx, &ws)?,
        };
        scenarios.push(s);
    }

    if wanted(opts, "multi-repo") {
        scenarios.push(multi_repo(&ctx, base.path())?);
    }
    if wanted(opts, "multi-workspace-idle") {
        scenarios.push(multi_workspace(&ctx, base.path())?);
    }
    if wanted(opts, "llm-answer") {
        scenarios.push(Scenario::skipped(
            "llm-answer",
            "needs a configured LLM provider; token accounting for answers and reviews is tracked in brokspace/brok-agent#23",
        ));
    }

    let mut system = sysinfo::System::new();
    system.refresh_memory();
    Ok(Report {
        schema_version: SCHEMA_VERSION,
        brok_version,
        environment: Environment {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            cpus: std::thread::available_parallelism().map_or(1, usize::from),
            total_memory_mb: system.total_memory() / (1024 * 1024),
        },
        fixture: FixtureInfo {
            size: opts.size.as_str().to_owned(),
            files: fixture.files,
            bytes: fixture.bytes,
            repositories: fixture.repositories,
        },
        mode: if opts.semantic { "semantic" } else { "text" }.to_owned(),
        scenarios,
    })
}

fn search(ctx: &Ctx<'_>, ws: &Path, fixture: &Fixture) -> Result<Scenario> {
    let before = ctx.sandbox.git_calls();
    let mut times: Vec<u64> = Vec::new();
    let (mut cpu, mut peak, mut read, mut write) = (0, 0, 0, 0);
    let mut failed = false;
    for query in &fixture.queries {
        let mut cmd = ctx.brok(ws, &["search", "--json", "-n", "8", query]);
        let m = measure::run_to_completion(&mut cmd)?;
        failed |= !m.success;
        times.push(m.wall.as_millis() as u64);
        cpu += m.cpu_ms;
        peak = peak.max(m.peak_rss);
        read += m.read_bytes;
        write += m.write_bytes;
    }
    times.sort_unstable();
    let total: u64 = times.iter().sum();
    let mut s = Scenario::new("search", if failed { Status::Failed } else { Status::Passed });
    s.wall_ms = Some(total);
    s.cpu_ms = Some(cpu);
    s.cpu_percent = Some(if total == 0 { 0.0 } else { (cpu as f64 / total as f64 * 1000.0).round() / 10.0 });
    s.peak_rss_bytes = Some(peak);
    s.io_read_bytes = Some(read);
    s.io_write_bytes = Some(write);
    s.git_subprocesses = ctx.sandbox.git_calls().zip(before).map(|(a, b)| a - b);
    s.extra.insert("runs".into(), times.len() as f64);
    s.extra.insert("p50_ms".into(), percentile(&times, 50) as f64);
    s.extra.insert("p95_ms".into(), percentile(&times, 95) as f64);
    s.reason = failed.then(|| "a search exited with an error".to_owned());
    Ok(s)
}

fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() * p).div_ceil(100)).clamp(1, sorted.len()) - 1]
}

fn idle_daemon(ctx: &Ctx<'_>, ws: &Path) -> Result<Scenario> {
    let mut args = vec!["run"];
    if !ctx.opts.with_agent {
        args.push("--no-shards");
    }
    let mut cmd = ctx.brok(ws, &args);
    let mut at_start = None;
    let (m, exited) =
        measure::run_for(&mut cmd, Duration::from_secs(3), Duration::from_secs(ctx.opts.idle_secs), &mut || {
            at_start = ctx.sandbox.git_calls();
        })?;
    let mut s = Scenario::new("idle-daemon", if exited.is_none() { Status::Passed } else { Status::Failed });
    if exited.is_some() {
        s.reason = Some("`brok run` exited by itself during the idle window".into());
    }
    fill(&mut s, &m);
    disk(&mut s, ws);
    s.git_subprocesses = ctx.sandbox.git_calls().zip(at_start).map(|(a, b)| a - b);
    s.extra.insert("window_secs".into(), ctx.opts.idle_secs as f64);
    // `peak_rss_bytes` includes the startup scan; this is what it settles at.
    s.extra.insert("steady_rss_bytes".into(), m.final_rss as f64);
    Ok(s)
}

fn multi_repo(ctx: &Ctx<'_>, base: &Path) -> Result<Scenario> {
    let ws = base.join("multi");
    let fx = fixture::generate(&ws, ctx.opts.size, 3)?;
    ctx.setup(&ws)?;
    let mut s = one_shot(ctx, "multi-repo", &ws, &["index"])?;
    s.extra.insert("repositories".into(), fx.repositories as f64);
    s.extra.insert("files".into(), fx.files as f64);
    Ok(s)
}

fn multi_workspace(ctx: &Ctx<'_>, base: &Path) -> Result<Scenario> {
    const COUNT: usize = 3;
    let mut cmds = Vec::new();
    for n in 0..COUNT {
        let ws = base.join(format!("many-{n}"));
        fixture::generate(&ws, Size::Small, 0)?;
        ctx.setup(&ws)?;
        ctx.run_ok(&ws, &["index"])?;
        let mut args = vec!["run"];
        if !ctx.opts.with_agent {
            args.push("--no-shards");
        }
        cmds.push(ctx.brok(&ws, &args));
    }
    let m = measure::run_many_for(&mut cmds, Duration::from_secs(3), Duration::from_secs(ctx.opts.idle_secs))?;
    let mut s = Scenario::new("multi-workspace-idle", if m.success { Status::Passed } else { Status::Failed });
    if !m.success {
        s.reason = Some("a daemon exited by itself during the idle window".into());
    }
    fill(&mut s, &m);
    s.extra.insert("workspaces".into(), COUNT as f64);
    s.extra.insert("steady_rss_bytes".into(), m.final_rss as f64);
    s.extra.insert("window_secs".into(), ctx.opts.idle_secs as f64);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        assert_eq!(percentile(&[], 95), 0);
        assert_eq!(percentile(&[5], 50), 5);
        let v: Vec<u64> = (1..=20).collect();
        assert_eq!(percentile(&v, 50), 10);
        assert_eq!(percentile(&v, 95), 19);
    }

    #[test]
    fn scenario_names_are_unique() {
        let mut names = ALL.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL.len());
    }
}
