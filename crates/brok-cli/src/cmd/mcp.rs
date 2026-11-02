//! `brok mcp`: a Model Context Protocol server over stdio.
//!
//! Coding agents (Claude Code, Cursor, VS Code, Codex, ...) start it with
//! `brok mcp` and get these tools:
//!
//! - `brok_search`: hybrid search over the workspace index, its linked
//!   workspaces and its memories;
//! - `brok_status`: what is indexed, linked and running;
//! - `brok_memory_save`, `brok_memory_search`, `brok_memory_recent`:
//!   memories kept between sessions (decisions, facts, handoffs).
//!
//! Messages are newline-delimited JSON-RPC 2.0 on stdin/stdout; logs go to
//! stderr only. Searches go through `brok run` when it is up (the model is
//! already loaded there) and otherwise through a read-only index opened once.

use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use brok_core::memory::{Kind, Memory, NewMemory, Scope};
use brok_core::{MemoryDirs, Workspace};
use brok_index::{Access, Index, SearchQuery};
use brok_sdk::payload::{SearchHit, SearchRequest, SearchResults};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::OnceCell;

use crate::daemon;

const PROTOCOL_VERSION: &str = "2025-06-18";
const INSTRUCTIONS: &str = "Brokspace indexes this workspace locally (code cut by syntax: functions, classes, sections), \
the workspaces it depends on (linked services and libraries, paths like ../checkout/...) and the memories kept \
between sessions. Call brok_search with a natural-language question or identifier before grepping when you need to \
find where something is implemented or gather context; results cite path:start-end. At the start of a task, call \
brok_memory_recent to catch up on recent decisions and handoffs. When you make a decision, learn a non-obvious fact \
or stop with work left, save it with brok_memory_save (one memory per fact, never secrets). Use brok_status to check \
freshness and links.";

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
            "brok_memory_save" => self.memory_save(&arguments),
            "brok_memory_search" => self.memory_search(&arguments),
            "brok_memory_recent" => self.memory_recent(&arguments),
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
        for link in index.link_status().await {
            let state = link.error.unwrap_or_else(|| format!("{} files, {} chunks", link.files, link.chunks));
            let about = link.description.map(|d| format!(" — {d}")).unwrap_or_default();
            text.push_str(&format!("linked workspace {} ({}): {state}{about}\n", link.name, link.root));
        }
        if workspace.config().memory.enabled {
            let count = MemoryDirs::of(workspace.paths(), workspace.config()).list().len();
            text.push_str(&format!("memories: {count}\n"));
        } else {
            text.push_str("memories: off\n");
        }
        Ok(text)
    }

    fn memory_dirs(&self) -> Result<(&Workspace, MemoryDirs), String> {
        let workspace = self.workspace()?;
        if !workspace.config().memory.enabled {
            return Err("memory is off in this workspace (memory.enabled in brok.yml)".into());
        }
        Ok((workspace, MemoryDirs::of(workspace.paths(), workspace.config())))
    }

    fn memory_save(&self, arguments: &Value) -> Result<String, String> {
        let (_, dirs) = self.memory_dirs()?;
        let text = |key: &str| arguments.get(key).and_then(Value::as_str).map(str::to_owned);
        let title = text("title").filter(|t| !t.trim().is_empty()).ok_or("`title` is required")?;
        let kind = match text("kind") {
            Some(kind) => kind.parse::<Kind>()?,
            None => Kind::Note,
        };
        let scope = match text("scope") {
            Some(scope) => scope.parse::<Scope>()?,
            None => Scope::Private,
        };
        let tags = arguments
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| tags.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let new = NewMemory {
            id: text("id"),
            title,
            body: text("content").unwrap_or_default(),
            kind,
            tags,
            source: Some("mcp".into()),
        };
        let memory = dirs.save(scope, new).map_err(|e| e.to_string())?;
        Ok(format!(
            "Saved memory `{}` ({}, {}) as {}. Pass id `{}` to update it.",
            memory.title,
            memory.kind.as_str(),
            memory.scope.as_str(),
            memory.path.display(),
            memory.id
        ))
    }

    fn memory_search(&self, arguments: &Value) -> Result<String, String> {
        let (workspace, _) = self.memory_dirs()?;
        let query = arguments.get("query").and_then(Value::as_str).filter(|q| !q.trim().is_empty());
        let query = query.ok_or("`query` is required")?;
        let limit = arguments.get("limit").and_then(Value::as_u64).map_or(8, |l| l.clamp(1, 50) as usize);
        let hits = brok_index::search_memories(workspace, query, limit);
        if hits.is_empty() {
            return Ok(format!("No memories match `{query}`."));
        }
        Ok(hits.iter().map(render_memory_hit).collect())
    }

    fn memory_recent(&self, arguments: &Value) -> Result<String, String> {
        let (_, dirs) = self.memory_dirs()?;
        let limit = arguments.get("limit").and_then(Value::as_u64).map_or(10, |l| l.clamp(1, 50) as usize);
        let memories: Vec<Memory> = dirs.list().into_iter().take(limit).collect();
        if memories.is_empty() {
            return Ok("No memories yet. Save decisions, facts and handoffs with brok_memory_save.".into());
        }
        let mut text = String::new();
        for memory in memories {
            text.push_str(&format!(
                "## {} ({} · {} · {}, id `{}`)\n{}\n\n",
                memory.title,
                memory.kind.as_str(),
                memory.scope.as_str(),
                memory.updated.get(..10).unwrap_or(&memory.updated),
                memory.id,
                excerpt(&memory.body, 600)
            ));
        }
        Ok(text)
    }
}

fn render_memory_hit(hit: &SearchHit) -> String {
    let body = hit.content.split_once("\n\n").map_or(hit.content.as_str(), |(_, body)| body);
    format!(
        "## {} ({}) — {}\n{}\n\n",
        hit.symbol.as_deref().unwrap_or_default(),
        hit.signature.as_deref().unwrap_or_default(),
        hit.path,
        excerpt(body, 1_200)
    )
}

/// The first `max` characters of `text`.
fn excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.trim().to_owned();
    }
    format!("{}…", text.chars().take(max).collect::<String>().trim_end())
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
            "description": "Show what the Brokspace index covers (partitions, files, chunks), the linked workspaces, \
    the memories and whether `brok run` is active.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "brok_memory_save",
            "description": "Remember something for later sessions (yours, the user's and other agents'): a decision \
    and why, a non-obvious fact about the code or system, or a handoff (where work stopped, what failed, what is next). \
    One memory per fact, concise Markdown. Never store secrets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "One line summary, e.g. 'Orders are idempotent per cart id'." },
                    "content": { "type": "string", "description": "Details in Markdown: context, reasons, file paths." },
                    "kind": { "type": "string", "enum": ["note", "decision", "fact", "handoff"], "description": "Default note." },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "scope": {
                        "type": "string",
                        "enum": ["private", "shared", "global"],
                        "description": "private (default): this workspace on this machine; shared: committed with the \
    repository, for the team; global: every workspace of this user (personal preferences)."
                    },
                    "id": { "type": "string", "description": "Id of a memory to replace (from a previous save or search)." }
                },
                "required": ["title"]
            }
        },
        {
            "name": "brok_memory_search",
            "description": "Search the memories of this workspace, the user's global ones and the linked workspaces' \
    (decisions, facts, earlier sessions). brok_search includes the best memories too.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }
        },
        {
            "name": "brok_memory_recent",
            "description": "The most recent memories (decisions, handoffs, sessions): call it at the start of a task \
    to catch up on where work left off.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Default 10." } }
            }
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
        assert_eq!(
            names,
            vec!["brok_search", "brok_status", "brok_memory_save", "brok_memory_search", "brok_memory_recent"]
        );

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
