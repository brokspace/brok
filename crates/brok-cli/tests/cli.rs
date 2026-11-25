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

#[test]
fn doctor_reports_the_source_control_provider_and_members() {
    let root = tempfile::tempdir().unwrap();
    let (web, plain) = (root.path().join("web"), root.path().join("plain"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() {}\n");
    std::fs::create_dir_all(&plain).unwrap();

    let doctor = |dir: &Path| json(&brok(dir, &["doctor", "--json"]));
    let names = |checks: &Value, name: &str| -> Vec<String> {
        checks
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["name"] == name)
            .map(|c| c["detail"].as_str().unwrap().to_owned())
            .collect()
    };
    let before = doctor(&web);
    assert!(names(&before, "source control")[0].starts_with("git (contract v1"), "{before}");
    assert!(names(&before, "member").is_empty());

    assert!(brok(&web, &["workspace", "add", "../plain"]).status.success());
    let after = doctor(&web);
    assert!(names(&after, "member")[0].contains("no Brokspace of its own"), "{after}");
}

fn git_repo_with(dir: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), body).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);
}

#[test]
fn a_git_url_member_is_cloned_indexed_searched_and_removed() {
    let root = tempfile::tempdir().unwrap();
    let (web, remote) = (root.path().join("web"), root.path().join("remote-api"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() {}\n");
    git_repo_with(&remote, "billing.rs", "fn compute_invoice_total() { /* invoice total */ }\n");
    let url = format!("file://{}", remote.display());

    let added = json(&brok(&web, &["workspace", "add", &url, "--id", "api", "--json"]));
    assert_eq!(added["added"], true);
    assert!(added["sync"]["error"].is_null(), "{added}");
    assert!(web.join(".brokspace/members/api/src/billing.rs").is_file());
    // The URL is what brok.yml records, never a path to the clone.
    let yml = std::fs::read_to_string(web.join(".brokspace/brok.yml")).unwrap();
    assert!(yml.contains("git:") && yml.contains("remote-api"), "{yml}");

    let all = json(&brok(&web, &["search", "invoice", "total", "--scope", "workspace", "--json"]));
    assert_eq!(all["hits"][0]["member"], "api", "{all}");
    assert_eq!(all["hits"][0]["path"], "billing.rs");
    // Not searched unless asked.
    assert!(json(&brok(&web, &["search", "invoice", "total", "--json"]))["hits"].as_array().unwrap().is_empty());

    // Registering again changes nothing; a sync pulls and indexes again.
    assert_eq!(json(&brok(&web, &["workspace", "add", &url, "--json"]))["added"], false);
    let synced = json(&brok(&web, &["workspace", "sync", "--json"]));
    assert_eq!(synced[0]["id"], "api");
    assert!(synced[0]["error"].is_null(), "{synced}");

    let status = json(&brok(&web, &["status", "--json"]));
    assert_eq!(status["members"][0]["state"], "repository");
    assert!(status["members"][0]["git"].as_str().unwrap().ends_with("remote-api"));

    // Removing deletes what brok made, and nothing else.
    assert!(brok(&web, &["workspace", "remove", "api"]).status.success());
    assert!(!web.join(".brokspace/members/api").exists());
    assert!(remote.join("billing.rs").is_file());
}

#[test]
fn a_git_member_with_credentials_or_a_bad_branch_is_refused_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    workspace_with(&web, "ui.rs", "fn render_dashboard() {}\n");
    let before = std::fs::read_to_string(web.join(".brokspace/brok.yml")).unwrap();

    let out = brok(&web, &["workspace", "add", "https://user:hunter2@example.com/acme/api.git"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("credentials") && !stderr.contains("hunter2"), "{stderr}");

    // --no-fetch registers without the network; --branch needs a URL.
    assert!(!brok(&web, &["workspace", "add", "../nowhere", "--branch", "x"]).status.success());
    assert_eq!(std::fs::read_to_string(web.join(".brokspace/brok.yml")).unwrap(), before);

    let lazy = json(&brok(&web, &["workspace", "add", "https://example.com/acme/api.git", "--no-fetch", "--json"]));
    assert_eq!(lazy["added"], true);
    let status = json(&brok(&web, &["workspace", "status", "--json"]));
    assert_eq!(status[0]["state"], "not_cloned");
    let search = json(&brok(&web, &["search", "dashboard", "--scope", "workspace", "--json"]));
    assert!(search["skipped_members"][0].as_str().unwrap().contains("not cloned yet"), "{search}");
}

/// A shard asks the index service for `scope: workspace` and gets the members' hits.
#[cfg(unix)]
#[test]
fn the_index_service_widens_a_search_to_the_members() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    use brok_sdk::payload::{SearchRequest, SearchResults, SearchScope};
    use brok_sdk::{BrokClient, ConnectOptions};

    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = tempfile::tempdir().unwrap();
    let (web, api) = (root.path().join("web"), root.path().join("api"));
    workspace_with(&web, "ui.rs", "fn render_dashboard() { /* draws the dashboard widgets */ }\n");
    workspace_with(&api, "billing.rs", "fn compute_invoice_total() { /* invoice total with taxes */ }\n");
    let add = brok(&web, &["workspace", "add", "../api", "--json"]);
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_brok"))
            .args(["run", "--no-shards"])
            .current_dir(&web)
            .env("BROK_MODELS_DIR", web.join(".models"))
            .env("BROK_CONFIG_DIR", web.join(".config"))
            .env("NO_COLOR", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("brok run starts"),
    );

    let workspace = brok_core::Workspace::open(&web).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(60);
        let client = loop {
            if let Ok(token) = std::fs::read_to_string(workspace.paths().operator_token()) {
                let options = ConnectOptions::new("test", "0.0.0", token.trim());
                if let Ok((client, _inbox)) = BrokClient::connect(&workspace.endpoint(), options).await {
                    break client;
                }
            }
            assert!(Instant::now() < deadline, "the daemon did not start");
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        let ask = |scope| {
            let client = client.clone();
            async move {
                // The index service answers once it has loaded; a loaded CI runner can take a while,
                // so a timeout is retried until the deadline instead of failing the test.
                let request = SearchRequest::new("invoice total").scope(scope);
                loop {
                    match client.call::<SearchRequest, SearchResults>(&request, Duration::from_secs(10)).await {
                        Ok(results) => break results,
                        Err(error) => {
                            assert!(Instant::now() < deadline, "the index service did not answer: {error}");
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
            }
        };
        let own = ask(SearchScope::Repository).await;
        assert!(own.hits.is_empty(), "this repository has nothing about invoices");

        let wide = ask(SearchScope::Workspace).await;
        assert_eq!(wide.hits.len(), 1, "{:?}", wide.hits.iter().map(|h| &h.path).collect::<Vec<_>>());
        assert_eq!(wide.hits[0].path, "api:billing.rs");
    });
}
