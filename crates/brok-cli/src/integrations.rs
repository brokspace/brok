//! Coding agent integrations: skills folders and MCP registration.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brok_skills::Catalog;
use serde_json::{Value, json};

/// Coding agents Brokspace can integrate with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, clap::ValueEnum)]
pub enum Agent {
    /// Claude Code: `.claude/skills/` and `.mcp.json`.
    Claude,
    /// The portable `.agents/skills/` folder (Codex, Cursor, Copilot and others).
    Agents,
    /// Cursor: reads `.agents/skills/`; MCP in `.cursor/mcp.json`.
    Cursor,
    /// VS Code with GitHub Copilot: MCP in `.vscode/mcp.json`.
    Vscode,
    /// OpenAI Codex: reads `.agents/skills/`; MCP in `~/.codex/config.toml`.
    Codex,
}

impl Agent {
    /// Every agent.
    pub const ALL: [Agent; 5] = [Agent::Claude, Agent::Agents, Agent::Cursor, Agent::Vscode, Agent::Codex];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Agents => "Portable .agents/skills (Codex, Cursor, Copilot, ...)",
            Agent::Cursor => "Cursor",
            Agent::Vscode => "VS Code / GitHub Copilot",
            Agent::Codex => "OpenAI Codex",
        }
    }

    /// Where this agent reads skills from (project or user level).
    pub fn skills_dir(self, root: &Path, global: bool) -> PathBuf {
        let base = if global { home_dir().unwrap_or_else(|| root.to_path_buf()) } else { root.to_path_buf() };
        match self {
            Agent::Claude => base.join(".claude").join("skills"),
            Agent::Agents | Agent::Cursor | Agent::Vscode | Agent::Codex => base.join(".agents").join("skills"),
        }
    }
}

/// Agents that look present in this workspace or on this machine. The
/// portable `.agents/skills/` target is always included.
pub fn detect(root: &Path) -> Vec<Agent> {
    let mut agents = vec![Agent::Agents];
    if root.join(".claude").is_dir() || root.join("CLAUDE.md").is_file() || on_path("claude") {
        agents.push(Agent::Claude);
    }
    if root.join(".cursor").is_dir() || on_path("cursor") {
        agents.push(Agent::Cursor);
    }
    if root.join(".vscode").is_dir() || on_path("code") {
        agents.push(Agent::Vscode);
    }
    if on_path("codex") || home_dir().is_some_and(|h| h.join(".codex").is_dir()) {
        agents.push(Agent::Codex);
    }
    agents.sort();
    agents
}

/// Exports every skill of `catalog` into each distinct directory. Returns
/// the directories written with the number of skills in each.
pub fn export_skills(catalog: &Catalog, dirs: &[PathBuf]) -> Result<Vec<(PathBuf, usize)>> {
    let unique: BTreeSet<&PathBuf> = dirs.iter().collect();
    let mut written = Vec::new();
    for dir in unique {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut count = 0;
        for skill in catalog.iter() {
            skill.export_to(dir)?;
            count += 1;
        }
        written.push((dir.clone(), count));
    }
    Ok(written)
}

/// Result of registering the MCP server for one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpSetup {
    /// A project file was created or updated.
    Written(PathBuf),
    /// The agent keeps MCP servers in user settings: show this snippet.
    Manual { file: &'static str, snippet: String },
}

/// Registers `brok mcp` with an agent.
pub fn configure_mcp(root: &Path, agent: Agent) -> Result<Option<McpSetup>> {
    let server = |typed: bool| {
        let mut entry = json!({ "command": "brok", "args": ["mcp"] });
        if typed {
            entry["type"] = json!("stdio");
        }
        entry
    };
    let written = match agent {
        Agent::Agents => return Ok(None),
        Agent::Codex => {
            let snippet = "[mcp_servers.brokspace]\ncommand = \"brok\"\nargs = [\"mcp\"]".to_owned();
            return Ok(Some(McpSetup::Manual { file: "~/.codex/config.toml", snippet }));
        }
        Agent::Claude => merge_server(&root.join(".mcp.json"), "mcpServers", server(true))?,
        Agent::Cursor => merge_server(&root.join(".cursor").join("mcp.json"), "mcpServers", server(false))?,
        Agent::Vscode => merge_server(&root.join(".vscode").join("mcp.json"), "servers", server(true))?,
    };
    Ok(Some(McpSetup::Written(written)))
}

/// Adds (or replaces) the `brokspace` entry under `key`, keeping every other
/// server and setting in the file.
fn merge_server(path: &Path, key: &str, entry: Value) -> Result<PathBuf> {
    let mut document: Value = match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).with_context(|| {
            format!("{} is not valid JSON; fix it or add the brokspace server by hand", path.display())
        })?,
        _ => json!({}),
    };
    let Some(object) = document.as_object_mut() else {
        bail!("{} must contain a JSON object", path.display());
    };
    let servers = object.entry(key).or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        bail!("`{key}` in {} must be an object", path.display());
    };
    servers.insert("brokspace".to_owned(), entry);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    brok_core::fsutil::write_atomic(path, format!("{}\n", serde_json::to_string_pretty(&document)?).as_bytes())?;
    Ok(path.to_path_buf())
}

/// Whether the agent's MCP registration for brokspace exists.
pub fn mcp_configured(root: &Path, agent: Agent) -> Option<bool> {
    let (path, key) = match agent {
        Agent::Claude => (root.join(".mcp.json"), "mcpServers"),
        Agent::Cursor => (root.join(".cursor").join("mcp.json"), "mcpServers"),
        Agent::Vscode => (root.join(".vscode").join("mcp.json"), "servers"),
        Agent::Agents | Agent::Codex => return None,
    };
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some(value.get(key).and_then(|s| s.get("brokspace")).is_some())
}

/// Whether an executable is reachable through `PATH`.
pub fn on_path(program: &str) -> bool {
    let names: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"].iter().map(|ext| format!("{program}.{ext}")).collect()
    } else {
        vec![program.to_owned()]
    };
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| names.iter().any(|n| dir.join(n).is_file())))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_entries_merge_without_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{"other":{"command":"x"}},"keep":1}"#).unwrap();

        configure_mcp(root, Agent::Claude).unwrap();
        configure_mcp(root, Agent::Claude).unwrap(); // idempotent
        let value: Value = serde_json::from_str(&std::fs::read_to_string(root.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(value["mcpServers"]["other"]["command"], "x");
        assert_eq!(value["mcpServers"]["brokspace"]["args"][0], "mcp");
        assert_eq!(value["keep"], 1);
        assert_eq!(mcp_configured(root, Agent::Claude), Some(true));

        configure_mcp(root, Agent::Vscode).unwrap();
        let vscode: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(".vscode/mcp.json")).unwrap()).unwrap();
        assert_eq!(vscode["servers"]["brokspace"]["type"], "stdio");
        assert!(matches!(configure_mcp(root, Agent::Codex).unwrap(), Some(McpSetup::Manual { .. })));

        std::fs::write(root.join(".cursor-broken.json"), "{").unwrap();
        assert!(merge_server(&root.join(".cursor-broken.json"), "mcpServers", json!({})).is_err());
    }

    #[test]
    fn exports_builtin_skills_once_per_directory() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::builtin();
        let dirs = [Agent::Agents, Agent::Cursor, Agent::Claude].map(|a| a.skills_dir(dir.path(), false));
        let written = export_skills(&catalog, &dirs).unwrap();
        assert_eq!(written.len(), 2, "cursor shares .agents/skills");
        assert!(dir.path().join(".claude/skills/tdd/SKILL.md").is_file());
        assert!(dir.path().join(".agents/skills/brok-context/agents/openai.yaml").is_file());
    }
}
