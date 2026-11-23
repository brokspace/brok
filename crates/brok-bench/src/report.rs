//! The result file: a versioned JSON document with numbers only.
//!
//! Nothing in a report comes from the benchmarked sources, prompts or
//! answers: the fixtures are synthetic, and a report holds scenario names,
//! counts, sizes and timings. A scenario that did not run is `skipped` with
//! a reason, never `passed`.

use serde::{Deserialize, Serialize};

/// Version of this file format. Bump it when a field changes meaning.
pub const SCHEMA_VERSION: u32 = 1;

/// How a scenario ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// It ran and the command succeeded.
    Passed,
    /// It ran and the command failed.
    Failed,
    /// It did not run (see `reason`).
    Skipped,
}

/// Measurements of one scenario. Absent values were not measurable on this
/// platform or for this scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Stable name, for example `index-initial`.
    pub name: String,
    /// How it ended.
    pub status: Status,
    /// Why it was skipped or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Wall-clock time of the measured part, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
    /// CPU time (user + system) of brok and its children, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_ms: Option<u64>,
    /// Average CPU use of the measured part, in percent of one core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f64>,
    /// Peak resident memory of brok and its children together, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_rss_bytes: Option<u64>,
    /// Bytes read from disk, as reported by the OS (0 where unsupported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io_read_bytes: Option<u64>,
    /// Bytes written to disk, as reported by the OS (0 where unsupported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io_write_bytes: Option<u64>,
    /// `git` processes started during the measured part (Unix only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_subprocesses: Option<u64>,
    /// Size of `.brokspace/databases` after the scenario, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_bytes: Option<u64>,
    /// Size of the whole `.brokspace` folder after the scenario, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_state_bytes: Option<u64>,
    /// Other counters of the scenario (`runs`, `p95_ms`, `workspaces`, ...).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: std::collections::BTreeMap<String, f64>,
}

impl Scenario {
    /// A scenario with no measurements yet.
    pub fn new(name: &str, status: Status) -> Self {
        Self {
            name: name.to_owned(),
            status,
            reason: None,
            wall_ms: None,
            cpu_ms: None,
            cpu_percent: None,
            peak_rss_bytes: None,
            io_read_bytes: None,
            io_write_bytes: None,
            git_subprocesses: None,
            index_bytes: None,
            workspace_state_bytes: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    /// A scenario that was not run.
    pub fn skipped(name: &str, reason: &str) -> Self {
        Self { reason: Some(reason.to_owned()), ..Self::new(name, Status::Skipped) }
    }
}

/// The machine the numbers come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    /// Operating system (`linux`, `macos`, `windows`).
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Logical CPUs.
    pub cpus: usize,
    /// Installed memory, in MiB.
    pub total_memory_mb: u64,
}

/// The synthetic workspace that was measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureInfo {
    /// `small`, `medium` or `large`.
    pub size: String,
    /// Source files in the workspace (all repositories).
    pub files: usize,
    /// Their total size, in bytes.
    pub bytes: u64,
    /// Git repositories in the workspace root included.
    pub repositories: usize,
}

/// A complete benchmark run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Format version, see [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Output of `brok --version`.
    pub brok_version: String,
    /// Machine description.
    pub environment: Environment,
    /// Workspace measured.
    pub fixture: FixtureInfo,
    /// Search mode: `text` (BM25 only) or `semantic`.
    pub mode: String,
    /// One entry per scenario, in execution order.
    pub scenarios: Vec<Scenario>,
}

impl Report {
    /// The scenario called `name`.
    pub fn scenario(&self, name: &str) -> Option<&Scenario> {
        self.scenarios.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn keys(value: &serde_json::Value, out: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    // `extra` is a free-form map of counters: its names are not part of the schema.
                    if k != "extra" {
                        out.insert(k.clone());
                        keys(v, out);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| keys(v, out)),
            _ => {}
        }
    }

    /// A report only ever holds numbers, fixed names and a version string.
    /// Pinning the key set makes a new field, which could carry content, a
    /// deliberate change.
    #[test]
    fn the_report_has_exactly_these_fields() {
        let mut full = Scenario::new("x", Status::Passed);
        full.reason = Some("r".into());
        full.wall_ms = Some(1);
        full.cpu_ms = Some(1);
        full.cpu_percent = Some(1.0);
        full.peak_rss_bytes = Some(1);
        full.io_read_bytes = Some(1);
        full.io_write_bytes = Some(1);
        full.git_subprocesses = Some(1);
        full.index_bytes = Some(1);
        full.workspace_state_bytes = Some(1);
        full.extra.insert("runs".into(), 1.0);
        let report = Report {
            schema_version: SCHEMA_VERSION,
            brok_version: "brok 0".into(),
            environment: Environment { os: "linux".into(), arch: "x86_64".into(), cpus: 1, total_memory_mb: 1 },
            fixture: FixtureInfo { size: "small".into(), files: 1, bytes: 1, repositories: 1 },
            mode: "text".into(),
            scenarios: vec![full],
        };
        let mut found = BTreeSet::new();
        keys(&serde_json::to_value(&report).unwrap(), &mut found);
        let expected: BTreeSet<String> = [
            "schema_version",
            "brok_version",
            "environment",
            "os",
            "arch",
            "cpus",
            "total_memory_mb",
            "fixture",
            "size",
            "files",
            "bytes",
            "repositories",
            "mode",
            "scenarios",
            "name",
            "status",
            "reason",
            "wall_ms",
            "cpu_ms",
            "cpu_percent",
            "peak_rss_bytes",
            "io_read_bytes",
            "io_write_bytes",
            "git_subprocesses",
            "index_bytes",
            "workspace_state_bytes",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn a_skipped_scenario_says_why_and_carries_no_numbers() {
        let s = Scenario::skipped("llm-answer", "needs an LLM");
        assert_eq!(s.status, Status::Skipped);
        assert_eq!(s.reason.as_deref(), Some("needs an LLM"));
        assert!(s.wall_ms.is_none() && s.peak_rss_bytes.is_none());
    }
}
