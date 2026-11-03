//! Local usage metrics: what brok did, how long it took, what failed.
//!
//! One JSON object per line in `.brokspace/metrics/<YYYY-MM>.jsonl`, written
//! by every brok process of the workspace (appends of one short line, so
//! processes do not interleave). Recording is best effort: it never fails or
//! slows down the command it measures. Nothing leaves the machine;
//! `brok stats` reads the files, and `brok bug` summarizes them in a report.
//!
//! ```json
//! {"at":"2026-11-03T10:12:01Z","kind":"search","ms":42,"hits":8,"semantic":true}
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::MetricsConfig;
use crate::workspace::WorkspacePaths;

/// Longest message kept in an `error` event.
const MAX_MESSAGE: usize = 300;

/// One recorded event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// When (RFC 3339, UTC).
    pub at: String,
    /// What: `search`, `answer`, `index`, `memory`, `link-sync`, `error`, ...
    pub kind: String,
    /// How long it took, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// Anything else, by kind (`hits`, `provider`, `ok`, `command`, ...).
    #[serde(flatten)]
    pub data: Map<String, Value>,
}

impl Event {
    /// A field of `data` as text.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(Value::as_str)
    }

    /// A field of `data` as a number.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.data.get(key).and_then(Value::as_f64)
    }

    /// A field of `data` as a boolean.
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.data.get(key).and_then(Value::as_bool)
    }
}

/// Where a workspace's metrics go, and whether they are recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorder {
    dir: Option<PathBuf>,
}

impl Recorder {
    /// The recorder of a workspace (off when `metrics.enabled` is false).
    pub fn new(paths: &WorkspacePaths, config: &MetricsConfig) -> Self {
        Self { dir: config.enabled.then(|| paths.metrics.clone()) }
    }

    /// A recorder that records nothing.
    pub fn off() -> Self {
        Self { dir: None }
    }

    /// Whether events are recorded.
    pub fn is_on(&self) -> bool {
        self.dir.is_some()
    }

    /// Records an event; `data` must be a JSON object (anything else is
    /// dropped). Failures are logged at debug level and otherwise ignored.
    pub fn record(&self, kind: &str, duration: Option<Duration>, data: Value) {
        let Some(dir) = &self.dir else { return };
        let data = match data {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let event = Event {
            at: crate::memory::now_rfc3339(),
            kind: kind.to_owned(),
            ms: duration.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            data,
        };
        if let Err(e) = append(dir, &event) {
            tracing::debug!(error = %e, "metrics not recorded");
        }
    }

    /// Records a failed command: its name and the message, secrets masked.
    pub fn record_error(&self, command: &str, message: &str) {
        let mut message = crate::memory::mask_secrets(message.lines().next().unwrap_or_default());
        if message.len() > MAX_MESSAGE {
            let mut end = MAX_MESSAGE;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        self.record("error", None, serde_json::json!({ "command": command, "message": message }));
    }
}

fn append(dir: &Path, event: &Event) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let month = event.at.get(..7).unwrap_or("unknown");
    let mut line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{month}.jsonl")))?;
    file.write_all(line.as_bytes())
}

/// Events of the last `days` days, oldest first. Unreadable lines are skipped.
pub fn load(paths: &WorkspacePaths, days: u32) -> Vec<Event> {
    let since = since(days);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&paths.metrics)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "jsonl"));
    files.sort();
    let month = since.get(..7).unwrap_or_default().to_owned();
    let mut events = Vec::new();
    for file in files {
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        if stem < month.as_str() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        events.extend(
            text.lines()
                .filter_map(|line| serde_json::from_str::<Event>(line).ok())
                .filter(|event| event.at.as_str() >= since.as_str()),
        );
    }
    events.sort_by(|a, b| a.at.cmp(&b.at));
    events
}

/// Deletes the months entirely older than `config.retention_days`.
pub fn prune(paths: &WorkspacePaths, config: &MetricsConfig) {
    let cutoff = since(config.retention_days);
    let Some(month) = cutoff.get(..7) else { return };
    let Ok(entries) = std::fs::read_dir(&paths.metrics) else { return };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        if path.extension().is_some_and(|e| e == "jsonl") && stem.as_str() < month {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The moment `days` days ago, RFC 3339.
fn since(days: u32) -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let then = secs.saturating_sub(u64::from(days) * 86_400);
    crate::memory::rfc3339_at(UNIX_EPOCH + Duration::from_secs(then))
}

/// The `p`-th percentile (0 to 100) of `values` (nearest rank), if any.
pub fn percentile(values: &[u64], p: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, WorkspacePaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = WorkspacePaths::new(dir.path());
        (dir, paths)
    }

    #[test]
    fn record_and_load() {
        let (_dir, paths) = paths();
        let recorder = Recorder::new(&paths, &MetricsConfig::default());
        recorder.record("search", Some(Duration::from_millis(42)), serde_json::json!({ "hits": 8 }));
        recorder.record_error("ask", "connection refused: token=abcdef123456 and more\nsecond line");
        recorder.record("ignored-data", None, Value::from(3));

        let events = load(&paths, 1);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, "search");
        assert_eq!(events[0].ms, Some(42));
        assert_eq!(events[0].number("hits"), Some(8.0));
        let error = &events[1];
        assert_eq!(error.text("command"), Some("ask"));
        let message = error.text("message").unwrap();
        assert!(!message.contains("abcdef123456") && !message.contains("second line"), "{message}");
        assert!(events[2].data.is_empty());
    }

    #[test]
    fn off_records_nothing() {
        let (_dir, paths) = paths();
        let config = MetricsConfig { enabled: false, ..MetricsConfig::default() };
        Recorder::new(&paths, &config).record("search", None, Value::Null);
        Recorder::off().record("search", None, Value::Null);
        assert!(load(&paths, 30).is_empty());
        assert!(!paths.metrics.exists());
    }

    #[test]
    fn prune_old_months() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.metrics).unwrap();
        std::fs::write(paths.metrics.join("2001-01.jsonl"), "{\"at\":\"2001-01-02T00:00:00Z\",\"kind\":\"search\"}\n")
            .unwrap();
        Recorder::new(&paths, &MetricsConfig::default()).record("search", None, Value::Null);
        assert_eq!(load(&paths, 36_500).len(), 2);
        prune(&paths, &MetricsConfig::default());
        assert!(!paths.metrics.join("2001-01.jsonl").exists());
        assert_eq!(load(&paths, 36_500).len(), 1);
    }

    #[test]
    fn percentiles() {
        let values = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(percentile(&values, 50.0), Some(50));
        assert_eq!(percentile(&values, 95.0), Some(100));
        assert_eq!(percentile(&[7], 95.0), Some(7));
        assert_eq!(percentile(&[], 50.0), None);
    }
}
