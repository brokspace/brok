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

fn workspace_with(dir: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), body).unwrap();
    let init =
        brok(dir, &["init", "--yes", "--no-agent", "--skip-model", "--no-llm", "--no-integrations", "--no-index"]);
    assert!(init.status.success(), "init failed: {}", String::from_utf8_lossy(&init.stderr));
    let index = brok(dir, &["index"]);
    assert!(index.status.success(), "index failed: {}", String::from_utf8_lossy(&index.stderr));
}

#[test]
fn scoped_search_reaches_members_only_when_asked() {
    let root = tempfile::tempdir().unwrap();
    let (web, api) = (root.path().join("web"), root.path().join("api"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() { /* draws the dashboard widgets */ }\n");
    workspace_with(&api, "billing.rs", "fn compute_invoice_total() { /* invoice total with taxes */ }\n");
    let add = brok(&web, &["workspace", "add", "../api", "--json"]);
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    // Default scope: this repository only, unchanged output.
    let own = json(&brok(&web, &["search", "invoice", "total", "--json"]));
    assert!(own["hits"].as_array().unwrap().is_empty());
    assert!(own.get("skipped_members").is_none());

    let all = json(&brok(&web, &["search", "invoice", "total", "--scope", "workspace", "--json"]));
    let hits = all["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{all}");
    assert_eq!(hits[0]["member"], "api");
    assert_eq!(hits[0]["path"], "billing.rs");

    let text = brok(&web, &["search", "invoice", "total", "--scope", "workspace"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("api:billing.rs"));
}

#[test]
fn scoped_search_names_members_it_could_not_search() {
    let root = tempfile::tempdir().unwrap();
    let (web, plain) = (root.path().join("web"), root.path().join("plain"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() {}\n");
    std::fs::create_dir_all(&plain).unwrap();
    let add = brok(&web, &["workspace", "add", "../plain", "--json"]);
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    let all = json(&brok(&web, &["search", "dashboard", "--scope", "workspace", "--json"]));
    assert!(all["hits"][0]["member"].is_null());
    assert!(all["skipped_members"][0].as_str().unwrap().starts_with("plain:"));
}

#[cfg(unix)]
#[test]
fn a_closed_pipe_ends_quietly() {
    use std::io::Read;
    use std::process::Stdio;

    let dir = tempfile::tempdir().unwrap();
    // Far more output than a pipe buffers, so the write fails once the reader is gone.
    let line = format!("fn item() {{ /* widget gadget {} */ }}\n", "x".repeat(200));
    workspace_with(dir.path(), "a.rs", &line.repeat(400));
    let mut child = Command::new(env!("CARGO_BIN_EXE_brok"))
        .args(["search", "widget", "gadget", "--full", "-n", "50"])
        .current_dir(dir.path())
        .env("BROK_MODELS_DIR", dir.path().join(".models"))
        .env("BROK_CONFIG_DIR", dir.path().join(".config"))
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read a little, then close the pipe like `| head -1`.
    let mut first = [0u8; 16];
    child.stdout.take().unwrap().read_exact(&mut first).unwrap();
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    let status = child.wait().unwrap();
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    assert!(status.success(), "status {status:?}, stderr: {stderr}");
}

#[test]
fn status_lists_members_only_in_a_meta_workspace() {
    let root = tempfile::tempdir().unwrap();
    let (web, api) = (root.path().join("web"), root.path().join("api"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() {}\n");
    workspace_with(&api, "billing.rs", "fn compute_invoice_total() {}\n");

    // Unchanged for a single repository.
    assert!(json(&brok(&web, &["status", "--json"])).get("members").is_none());

    assert!(brok(&web, &["workspace", "add", "../api"]).status.success());
    let status = json(&brok(&web, &["status", "--json"]));
    assert_eq!(status["members"][0]["id"], "api");
    assert_eq!(status["members"][0]["state"], "directory");
    assert_eq!(status["members"][0]["has_brokspace"], true);
}

#[test]
fn mcp_search_reaches_members_when_the_agent_asks() {
    use std::io::Write;
    use std::process::Stdio;

    let root = tempfile::tempdir().unwrap();
    let (web, api) = (root.path().join("web"), root.path().join("api"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() {}\n");
    workspace_with(&api, "billing.rs", "fn compute_invoice_total() { /* invoice total */ }\n");
    assert!(brok(&web, &["workspace", "add", "../api"]).status.success());

    let call = |args: &str| -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_brok"))
            .arg("mcp")
            .current_dir(&web)
            .env("BROK_MODELS_DIR", web.join(".models"))
            .env("BROK_CONFIG_DIR", web.join(".config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"brok_search","arguments":{args}}}}}"#
        );
        child.stdin.take().unwrap().write_all(format!("{request}\n").as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let own = call(r#"{"query":"invoice total"}"#);
    assert!(own.contains("No results"), "{own}");
    let all = call(r#"{"query":"invoice total","scope":"workspace"}"#);
    assert!(all.contains("api:billing.rs"), "{all}");
}
