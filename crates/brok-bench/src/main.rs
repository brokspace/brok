//! `brok-bench`: reproducible resource benchmarks for brok.
//!
//! ```text
//! cargo build --release -p brok-cli --no-default-features
//! cargo run --release -p brok-bench -- run --brok target/release/brok --size small --out result.json
//! cargo run --release -p brok-bench -- compare benches/baseline/small-text.json result.json
//! ```
//!
//! See `docs/benchmarks.md`.

mod compare;
mod fixture;
mod measure;
mod report;
mod scenarios;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::fixture::Size;
use crate::report::{Report, Status};

#[derive(Parser)]
#[command(name = "brok-bench", version, about = "Reproducible resource benchmarks for brok")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the scenarios against a brok binary and write a JSON report.
    Run {
        /// The `brok` binary to measure (build it with `--release`).
        #[arg(long, default_value = "target/release/brok")]
        brok: PathBuf,
        /// Workspace size: small (~50 files), medium (~1,500) or large (~15,000).
        #[arg(long, default_value = "small")]
        size: String,
        /// Where to write the JSON report.
        #[arg(long, default_value = "bench-result.json")]
        out: PathBuf,
        /// Also measure semantic search (downloads the embedding model once).
        #[arg(long)]
        semantic: bool,
        /// Seconds of the idle measurement window.
        #[arg(long, default_value_t = 20)]
        idle_secs: u64,
        /// Let `brok init` install brok-agent (network); measures its git polling.
        #[arg(long)]
        with_agent: bool,
        /// Run only these scenarios (comma separated). See `list`.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Embedding model cache to use (default: a temporary one).
        #[arg(long)]
        models_dir: Option<PathBuf>,
    },
    /// Compare a report with a baseline; exit 1 on a regression.
    Compare {
        /// The reference report.
        baseline: PathBuf,
        /// The new report.
        current: PathBuf,
        /// Allowed growth of the index size and of git processes (fraction).
        #[arg(long, default_value_t = 0.10)]
        tolerance: f64,
        /// Also fail when peak memory grows by more than this fraction.
        #[arg(long)]
        memory_tolerance: Option<f64>,
    },
    /// Print the scenario names.
    List,
    /// Generate a fixture workspace and stop (for manual experiments).
    Fixture {
        /// Directory to create.
        dir: PathBuf,
        /// small, medium or large.
        #[arg(long, default_value = "small")]
        size: String,
        /// Extra nested repositories (and one submodule when above zero).
        #[arg(long, default_value_t = 0)]
        nested: usize,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::List => {
            for name in scenarios::ALL {
                println!("{name}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Fixture { dir, size, nested } => {
            let f = fixture::generate(&dir, Size::parse(&size)?, nested)?;
            println!("{} files, {} bytes, {} repositories in {}", f.files, f.bytes, f.repositories, dir.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Run { brok, size, out, semantic, idle_secs, with_agent, only, models_dir } => {
            for name in &only {
                if !scenarios::ALL.contains(&name.as_str()) {
                    bail!("unknown scenario `{name}` (see `brok-bench list`)");
                }
            }
            if idle_secs == 0 {
                bail!("--idle-secs must be at least 1");
            }
            let own_models = tempfile::tempdir().context("temporary model cache")?;
            let options = scenarios::Options {
                brok,
                size: Size::parse(&size)?,
                semantic,
                idle_secs,
                with_agent,
                only,
                models_dir: models_dir.unwrap_or_else(|| own_models.path().to_path_buf()),
            };
            let report = scenarios::run(&options)?;
            std::fs::write(&out, serde_json::to_string_pretty(&report)? + "\n")
                .with_context(|| format!("write {}", out.display()))?;
            print_summary(&report);
            eprintln!("report written to {}", out.display());
            let failed = report.scenarios.iter().any(|s| s.status == Status::Failed);
            Ok(if failed { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        Command::Compare { baseline, current, tolerance, memory_tolerance } => {
            let read = |p: &PathBuf| -> Result<Report> {
                serde_json::from_str(&std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?)
                    .with_context(|| format!("parse {}", p.display()))
            };
            let outcome =
                compare::compare(&read(&baseline)?, &read(&current)?, compare::Limits { tolerance, memory_tolerance })?;
            println!("{}", outcome.markdown());
            if outcome.failed() {
                eprintln!("regression: a gated metric got worse than allowed");
                return Ok(ExitCode::from(1));
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn print_summary(report: &Report) {
    let mib = |b: Option<u64>| b.map_or("-".to_owned(), |b| format!("{:.1}", b as f64 / (1024.0 * 1024.0)));
    let num = |v: Option<u64>| v.map_or("-".to_owned(), |v| v.to_string());
    println!(
        "{} on {}/{} ({} cpus), {} fixture ({} files, {} repos), {} search\n",
        report.brok_version,
        report.environment.os,
        report.environment.arch,
        report.environment.cpus,
        report.fixture.size,
        report.fixture.files,
        report.fixture.repositories,
        report.mode
    );
    println!(
        "| Scenario | Status | Wall ms | CPU ms | Peak RSS MiB | Git procs | Index MiB |\n|---|---|---:|---:|---:|---:|---:|"
    );
    for s in &report.scenarios {
        let status = match s.status {
            Status::Passed => "passed",
            Status::Failed => "FAILED",
            Status::Skipped => "skipped",
        };
        println!(
            "| {} | {status} | {} | {} | {} | {} | {} |",
            s.name,
            num(s.wall_ms),
            num(s.cpu_ms),
            mib(s.peak_rss_bytes),
            num(s.git_subprocesses),
            mib(s.index_bytes)
        );
    }
    for s in report.scenarios.iter().filter(|s| s.reason.is_some()) {
        println!("\n- {}: {}", s.name, s.reason.as_deref().unwrap_or_default());
    }
}
