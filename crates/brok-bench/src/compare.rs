//! Comparing two reports.
//!
//! Only metrics that do not depend on the machine's mood can fail a build:
//! the size of the index and the number of `git` processes. Memory can be
//! gated on request. Wall time and CPU time are shown but never fail, since
//! a shared CI runner makes them noisy.

use anyhow::{Result, bail};

use crate::report::{Report, Scenario, Status};

/// Limits for [`compare`].
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Allowed growth of deterministic metrics (index size, git processes).
    pub tolerance: f64,
    /// Also fail on peak memory growing by more than this fraction.
    pub memory_tolerance: Option<f64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self { tolerance: 0.10, memory_tolerance: None }
    }
}

/// How a metric affects the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Fails the comparison when it regresses.
    Gated,
    /// Shown only.
    Info,
}

/// One compared value.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Scenario name.
    pub scenario: String,
    /// Metric name.
    pub metric: &'static str,
    /// Baseline value.
    pub baseline: f64,
    /// Current value.
    pub current: f64,
    /// Whether it can fail the build.
    pub kind: Kind,
    /// Whether it is worse than allowed.
    pub regressed: bool,
}

impl Finding {
    /// Relative change, in percent.
    pub fn change_percent(&self) -> f64 {
        if self.baseline == 0.0 {
            if self.current == 0.0 { 0.0 } else { 100.0 }
        } else {
            (self.current / self.baseline - 1.0) * 100.0
        }
    }
}

/// The comparison result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    /// Every compared value.
    pub findings: Vec<Finding>,
    /// Scenarios that passed in the baseline and did not now.
    pub broken: Vec<String>,
}

impl Outcome {
    /// Whether the comparison should fail the build.
    pub fn failed(&self) -> bool {
        !self.broken.is_empty() || self.findings.iter().any(|f| f.regressed && f.kind == Kind::Gated)
    }

    /// A Markdown table, for the job summary.
    pub fn markdown(&self) -> String {
        let mut out =
            String::from("| Scenario | Metric | Baseline | Current | Change | |\n|---|---|---:|---:|---:|---|\n");
        for f in &self.findings {
            let mark = match (f.regressed, f.kind) {
                (true, Kind::Gated) => "FAIL",
                (true, Kind::Info) => "slower (info)",
                _ => "",
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {:+.1}% | {mark} |\n",
                f.scenario,
                f.metric,
                human(f.metric, f.baseline),
                human(f.metric, f.current),
                f.change_percent()
            ));
        }
        for name in &self.broken {
            out.push_str(&format!("| {name} | status | passed | not passed | | FAIL |\n"));
        }
        out
    }
}

fn human(metric: &str, v: f64) -> String {
    if metric.ends_with("bytes") { format!("{:.1} MiB", v / (1024.0 * 1024.0)) } else { format!("{v:.0}") }
}

/// Compares `current` against `baseline`. The two must describe the same
/// fixture and mode, or the numbers mean different things.
pub fn compare(baseline: &Report, current: &Report, limits: Limits) -> Result<Outcome> {
    if baseline.schema_version != current.schema_version {
        bail!(
            "schema versions differ ({} vs {}): regenerate the baseline",
            baseline.schema_version,
            current.schema_version
        );
    }
    if baseline.fixture.size != current.fixture.size || baseline.mode != current.mode {
        bail!(
            "not comparable: baseline is {}/{}, current is {}/{}",
            baseline.fixture.size,
            baseline.mode,
            current.fixture.size,
            current.mode
        );
    }
    let mut outcome = Outcome::default();
    for base in &baseline.scenarios {
        let Some(cur) = current.scenario(&base.name) else { continue };
        if base.status == Status::Passed && cur.status != Status::Passed {
            outcome.broken.push(base.name.clone());
            continue;
        }
        if base.status != Status::Passed || cur.status != Status::Passed {
            continue;
        }
        metrics(base, cur, limits, &mut outcome);
    }
    Ok(outcome)
}

fn metrics(base: &Scenario, cur: &Scenario, limits: Limits, out: &mut Outcome) {
    let mut add = |metric: &'static str, b: Option<f64>, c: Option<f64>, kind: Kind, tolerance: f64, min_abs: f64| {
        let (Some(b), Some(c)) = (b, c) else { return };
        if b == 0.0 && c == 0.0 {
            return; // nothing to compare, and a table full of zeros hides the rest
        }
        let regressed = c > b * (1.0 + tolerance) && c - b >= min_abs;
        out.findings.push(Finding { scenario: cur.name.clone(), metric, baseline: b, current: c, kind, regressed });
    };
    let n = |v: Option<u64>| v.map(|v| v as f64);
    add("index_bytes", n(base.index_bytes), n(cur.index_bytes), Kind::Gated, limits.tolerance, 4096.0);
    // A couple of extra `git` calls is timing noise; a new polling loop is not.
    add("git_subprocesses", n(base.git_subprocesses), n(cur.git_subprocesses), Kind::Gated, limits.tolerance, 3.0);
    let (kind, tolerance) = match limits.memory_tolerance {
        Some(t) => (Kind::Gated, t),
        None => (Kind::Info, 0.25),
    };
    add("peak_rss_bytes", n(base.peak_rss_bytes), n(cur.peak_rss_bytes), kind, tolerance, 8.0 * 1024.0 * 1024.0);
    add("cpu_ms", n(base.cpu_ms), n(cur.cpu_ms), Kind::Info, 0.5, 200.0);
    // The wall time of a daemon measured over a fixed window is that window.
    if !base.extra.contains_key("window_secs") {
        add("wall_ms", n(base.wall_ms), n(cur.wall_ms), Kind::Info, 0.5, 200.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Environment, FixtureInfo, SCHEMA_VERSION};

    fn report(scenarios: Vec<Scenario>) -> Report {
        Report {
            schema_version: SCHEMA_VERSION,
            brok_version: "brok 0.0.0".into(),
            environment: Environment { os: "linux".into(), arch: "x86_64".into(), cpus: 4, total_memory_mb: 8000 },
            fixture: FixtureInfo { size: "small".into(), files: 50, bytes: 1000, repositories: 1 },
            mode: "text".into(),
            scenarios,
        }
    }

    fn idx(index: u64, git: u64, rss_mb: u64) -> Scenario {
        let mut s = Scenario::new("index-initial", Status::Passed);
        s.index_bytes = Some(index);
        s.git_subprocesses = Some(git);
        s.peak_rss_bytes = Some(rss_mb * 1024 * 1024);
        s.wall_ms = Some(1000);
        s
    }

    #[test]
    fn same_numbers_pass() {
        let r = report(vec![idx(1_000_000, 10, 100)]);
        let out = compare(&r, &r, Limits::default()).unwrap();
        assert!(!out.failed());
        assert!(out.findings.iter().all(|f| !f.regressed));
    }

    #[test]
    fn a_bigger_index_fails() {
        let out =
            compare(&report(vec![idx(1_000_000, 10, 100)]), &report(vec![idx(1_300_000, 10, 100)]), Limits::default())
                .unwrap();
        assert!(out.failed());
        assert!(out.findings.iter().any(|f| f.metric == "index_bytes" && f.regressed));
    }

    #[test]
    fn a_few_more_git_calls_are_noise_but_a_loop_is_not() {
        let base = report(vec![idx(1_000_000, 0, 100)]);
        assert!(!compare(&base, &report(vec![idx(1_000_000, 2, 100)]), Limits::default()).unwrap().failed());
        assert!(compare(&base, &report(vec![idx(1_000_000, 12, 100)]), Limits::default()).unwrap().failed());
    }

    #[test]
    fn memory_only_gates_on_request() {
        let (b, c) = (report(vec![idx(1_000_000, 0, 100)]), report(vec![idx(1_000_000, 0, 200)]));
        assert!(!compare(&b, &c, Limits::default()).unwrap().failed());
        let strict = Limits { memory_tolerance: Some(0.5), ..Limits::default() };
        assert!(compare(&b, &c, strict).unwrap().failed());
    }

    #[test]
    fn a_scenario_that_stopped_passing_fails() {
        let mut broken = idx(1, 1, 1);
        broken.status = Status::Failed;
        let out = compare(&report(vec![idx(1_000_000, 0, 100)]), &report(vec![broken]), Limits::default()).unwrap();
        assert!(out.failed());
        assert_eq!(out.broken, vec!["index-initial"]);
    }

    #[test]
    fn different_fixtures_are_not_comparable() {
        let a = report(vec![]);
        let mut b = report(vec![]);
        b.fixture.size = "large".into();
        assert!(compare(&a, &b, Limits::default()).is_err());
    }

    #[test]
    fn zero_rows_and_fixed_window_wall_times_are_left_out() {
        let mut idle = Scenario::new("idle-daemon", Status::Passed);
        idle.wall_ms = Some(15_000);
        idle.git_subprocesses = Some(0);
        idle.extra.insert("window_secs".into(), 15.0);
        let r = report(vec![idle]);
        let out = compare(&r, &r, Limits::default()).unwrap();
        assert!(out.findings.is_empty(), "{:?}", out.findings);
    }

    #[test]
    fn markdown_lists_findings() {
        let r = report(vec![idx(1_000_000, 10, 100)]);
        let md = compare(&r, &r, Limits::default()).unwrap().markdown();
        assert!(md.contains("index_bytes") && md.starts_with("| Scenario"));
    }
}
