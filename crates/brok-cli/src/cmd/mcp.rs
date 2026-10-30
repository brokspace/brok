//! `brok mcp`: a Model Context Protocol server over stdio.
//!
//! Coding agents (Claude Code, Cursor, VS Code, Codex, ...) start it with
//! `brok mcp` and get two tools:
//!
//! - `brok_search`: hybrid search over the workspace index;
//! - `brok_status`: what is indexed and what is running.
//!
//! Messages are newline-delimited JSON-RPC 2.0 on stdin/stdout; logs go to
//! stderr only. Searches go through `brok run` when it is up (the model is
//! already loaded there) and otherwise through a read-only index opened once.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use brok_core::Workspace;
use brok_index::{Access, Index, SearchQuery};
use brok_sdk::payload::{SearchHit, SearchRequest, SearchResults};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::OnceCell;

use crate::daemon;

const PROTOCOL_VERSION: &str = "2025-06-18";
const INSTRUCTIONS: &str = "Brokspace indexes this workspace locally (code cut by syntax: functions, classes, sections). \
Call brok_search with a natural-language question or identifier before grepping when you need to find where \
something is implemented or gather context; results cite path:start-end. Use brok_status to check freshness.";

/// Arguments of `brok mcp`.
#[derive(Debug, clap::Args)]
pub struct Args {}

struct Server {
    workspace: Result<Workspace, String>,
    index: OnceCell<Result<Index, String>>,
}

pub async fn run(cwd: &Path, _args: Args) -> Result<()> {
    let server = Server { workspace: Workspace::discover(cwd).map_err(|e| e.to_string()), index: OnceCell::new() };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        // Windows shells piping into a process may prefix a byte order mark.
        let line = line.trim_start_matches('\u{feff}');
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(line) {
            Ok(message) => server.handle(message).await,
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(response) = response {
            stdout.write_all(format!("{response}\n").as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

impl Server {
    /// Returns `None` for notifications (no `id`).
    async fn handle(&self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned()?;
        let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "brokspace", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => Ok(self.call(&params).await),
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error(id, code, &message),
        })
    }

    async fn call(&self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let outcome = match name {
            "brok_search" => self.search(&arguments).await,
            "brok_status" => self.status().await,
            other => Err(format!("unknown tool `{other}`")),
        };
        match outcome {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(message) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
        }
    }

    fn workspace(&self) -> Result<&Workspace, String> {
        self.workspace.as_ref().map_err(|e| format!("{e}. Run `brok init` in the project root."))
    }

    async fn search(&self, arguments: &Value) -> Result<String, String> {
        let workspace = self.workspace()?;
        let query = arguments.get("query").and_then(Value::as_str).filter(|q| !q.trim().is_empty());
        let query = query.ok_or("`query` is required")?.to_owned();
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(workspace.config().search.limit, |l| l.clamp(1, 50) as usize);

        let results = if daemon::is_running(workspace).await {
            let (client, _inbox) = daemon::connect(workspace).await.map_err(|e| format!("{e:#}"))?;
            let request = SearchRequest { query: query.clone(), limit, focus: None };
            client
                .call::<SearchRequest, SearchResults>(&request, Duration::from_secs(60))
                .await
                .map_err(|e| e.to_string())?
        } else {
            let index = self
                .index
                .get_or_init(|| async {
                    let embedder = super::embedder_for_search(workspace);
                    Index::open(workspace, embedder, Access::ReadOnly).await.map_err(|e| format!("{e:#}"))
                })
                .await
                .as_ref()
                .map_err(Clone::clone)?;
            index
                .search(&SearchQuery { text: query.clone(), limit, focus: None })
                .await
                .map_err(|e| format!("{e:#}"))?
        };
        Ok(render(&query, &results))
    }

    async fn status(&self) -> Result<String, String> {
        let workspace = self.workspace()?;
        let index = Index::open(workspace, None, Access::ReadOnly).await.map_err(|e| format!("{e:#}"))?;
        let parts = index.status().await.map_err(|e| format!("{e:#}"))?;
        let mut text = format!(
            "workspace: {}\ndaemon: {}\nembeddings: {}\n",
            workspace.root().display(),
            if daemon::is_running(workspace).await { "running" } else { "stopped" },
            if workspace.config().embeddings.enabled { "on" } else { "off (text search only)" },
        );
        if parts.is_empty() {
            text.push_str("index: empty (run `brok index`)\n");
        }
        for p in parts {
            let path = if p.path.is_empty() { "." } else { p.path.as_str() };
            text.push_str(&format!("partition {} ({path}): {} files, {} chunks\n", p.id, p.files, p.chunks));
        }
        Ok(text)
    }
}

fn tools() -> Value {
    json!([
        {
            "name": "brok_search",
            "description": "Search this workspace's local index by meaning and by identifier (hybrid BM25 + embeddings). \
    Returns whole functions, classes or sections with path:start-end. Prefer it over grep for 'where is X implemented', \
    'how does Y work', or finding related code; use grep for exact strings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language question or identifier, e.g. 'retry policy for http calls' or 'parseConfig'." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Maximum number of results (default 8)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "brok_status",
            "description": "Show what the Brokspace index covers (partitions, files, chunks) and whether `brok run` is active.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn render(query: &str, results: &SearchResults) -> String {
    if results.hits.is_empty() {
        return format!("No results for `{query}`. The index may be empty or stale: run `brok index`.");
    }
    let mut text = String::new();
    for hit in &results.hits {
        text.push_str(&render_hit(hit));
    }
    if !results.semantic {
        text.push_str("(text search only: semantic search is off)\n");
    }
    text
}

fn render_hit(hit: &SearchHit) -> String {
    let symbol = hit.symbol.as_deref().map(|s| format!(" {s}")).unwrap_or_default();
    format!(
        "## {}:{}-{} ({}{symbol}, score {:.2})\n```{}\n{}\n```\n\n",
        hit.path,
        hit.start_line,
        hit.end_line,
        hit.kind,
        hit.score,
        hit.language,
        hit.content.trim_end()
    )
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server { workspace: Err("no workspace".into()), index: OnceCell::new() }
    }

    #[tokio::test]
    async fn handshake_and_tool_listing() {
        let s = server();
        let init = s
            .handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}))
            .await
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "brokspace");
        assert!(s.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await.is_none());

        let list = s.handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await.unwrap();
        let names: Vec<&str> =
            list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["brok_search", "brok_status"]);

        let unknown = s.handle(json!({"jsonrpc":"2.0","id":3,"method":"nope"})).await.unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn tool_errors_are_reported_as_content() {
        let s = server();
        let call = json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"brok_search","arguments":{"query":"x"}}});
        let response = s.handle(call).await.unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"].as_str().unwrap().contains("brok init"));
    }
}
