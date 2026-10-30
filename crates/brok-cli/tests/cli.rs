//! The machine-readable output editors rely on (`--json`), checked against
//! the real binary.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn brok(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_brok"))
        .args(args)
        .current_dir(dir)
        // Keep the test away from the user's models and defaults.
        .env("BROK_MODELS_DIR", dir.join(".models"))
        .env("BROK_CONFIG_DIR", dir.join(".config"))
        .env("NO_COLOR", "1")
        .output()
        .expect("brok runs")
}

fn json(output: &std::process::Output) -> Value {
    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON document")
}

#[test]
fn status_json_outside_a_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let status = json(&brok(dir.path(), &["status", "--json"]));
    assert_eq!(status["initialized"], false);
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn status_json_in_a_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    let init = brok(
        dir.path(),
        &["init", "--yes", "--no-agent", "--skip-model", "--no-llm", "--no-integrations", "--no-index"],
    );
    assert!(init.status.success(), "init failed: {}", String::from_utf8_lossy(&init.stderr));

    let status = json(&brok(dir.path(), &["status", "--json"]));
    assert_eq!(status["initialized"], true);
    assert_eq!(status["daemon"]["running"], false);
    assert_eq!(status["embeddings"]["enabled"], false);
    assert_eq!(status["llm"]["provider"], "ollama");
    assert!(status["index"].is_array());
    assert!(status["shards"].is_array());
    assert!(status["root"].as_str().is_some_and(|root| !root.is_empty()));
}
