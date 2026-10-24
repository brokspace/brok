# Integrations

Brokspace stays small and plugs into the tools you already use.

| Integration | How | Setup |
|---|---|---|
| Coding agents (Claude Code, Cursor, VS Code / Copilot, Codex) | MCP tools + skills | `brok setup agents` |
| Local LLMs (Ollama, LM Studio) | OpenAI-compatible API | `brok setup llm` |
| Hosted LLMs (Anthropic, DeepSeek, any OpenAI-compatible) | `llm.providers` in `brok.yml` | export the API key |
| Git and submodules | change tracking, hot/cold partitions | automatic |
| Team storage (S3, R2, MinIO, Git LFS) | index mirroring | `brok setup sync` |
| Anything else | write a shard | `brok shard new` |

## Coding agents

```sh
brok setup agents            # detects your agents and asks which to connect
brok setup agents -a claude -a cursor --yes
```

For each agent it does two things.

**1. MCP server.** Registers `brok mcp` (a stdio
[Model Context Protocol](https://modelcontextprotocol.io) server) so the
agent gets two tools:

| Tool | What it does |
|---|---|
| `brok_search` | Hybrid search over the local index; returns whole functions/classes with `path:start-end` |
| `brok_status` | What is indexed and whether `brok run` is active |

| Agent | File written |
|---|---|
| Claude Code | `.mcp.json` (`mcpServers.brokspace`) |
| Cursor | `.cursor/mcp.json` (`mcpServers.brokspace`) |
| VS Code / Copilot | `.vscode/mcp.json` (`servers.brokspace`) |
| Codex | prints the snippet for `~/.codex/config.toml` |

Existing entries in those files are preserved; only `brokspace` is added or
updated. When `brok run` is active, searches reuse its loaded model;
otherwise `brok mcp` opens the index read-only on demand.

**2. Skills.** Exports the [skills](skills.md) to `.claude/skills/` (Claude
Code) and `.agents/skills/` (Codex, Cursor, Copilot and others), so `/ask-brok`,
`/grill-me`, `tdd`, `code-review` and the rest are available in the agent.

Commit the generated files to give your whole team the same setup. `brok`
must be on `PATH` for agents to start `brok mcp`; `brok doctor` checks it.

## Local and hosted LLMs

See [local-llm.md](local-llm.md). Any OpenAI-compatible endpoint works:

```yaml
llm:
  default: ollama
  providers:
    ollama:   { base_url: http://localhost:11434/v1, model: qwen2.5-coder:7b }
    company:  { base_url: https://llm.internal.example.com/v1, model: gpt-oss-20b, api_key_env: COMPANY_LLM_KEY }
```

## Shards: integrations you build

When something is specific to your team (a ticketing system, a deploy
pipeline, a compliance check), write a shard instead of waiting for a core
feature: it reacts to workspace events and publishes its own. See
[shards.md](shards.md) and start with `brok shard new my-integration`.
