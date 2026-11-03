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
agent gets these tools:

| Tool | What it does |
|---|---|
| `brok_search` | Hybrid search over the local index, the [linked workspaces](memory.md#linked-workspaces) and the memories; returns whole functions/classes with `path:start-end` |
| `brok_status` | What is indexed and linked, how many memories, and whether `brok run` is active |
| `brok_memory_save` | Saves a [memory](memory.md) (decision, fact, note, handoff) for later sessions |
| `brok_memory_search` | Searches the memories of this workspace, your global ones and the linked workspaces' |
| `brok_memory_recent` | The latest memories: agents call it at the start of a task to catch up |

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

## Editors and scripts: JSON output

Editors (the Brokspace IDE among them), scripts and CI read brok through
`--json`. Shapes are stable within a minor version.

| Command | Output |
|---|---|
| `brok status --json` | One document: `version`, `initialized`, `root`, `daemon.running`, `backends`, `embeddings`, `llm` (`provider`, `model`, `base_url`), `index[]` (`tier`, `path`, `files`, `chunks`, `backend`, `semantic`), `shards[]` (`name`, `version`, `description`, `state`, `source`), `links[]` (as `brok link list --json`), `memory` (`enabled`, `sessions`, `global`, `counts.private`, `counts.shared`, `counts.global`). Outside a workspace: `{"version": ..., "initialized": false}`. |
| `brok search <query> --json` | One document: `hits[]` with `path`, `start_line`, `end_line`, `kind`, `symbol`, `score`, `content`, `partition` (`@<link>` for linked workspaces, `memory` for memories). |
| `brok skill list --json` | One array: `name`, `description`, `user_invoked`, `source`, `bucket` per skill. |
| `brok memory list --json` | One array of memories: `id`, `title`, `kind`, `scope`, `tags`, `created`, `updated`, `source`, `body`, `path`. `brok memory add/show ... --json` print one. |
| `brok memory search <query> --json` | One array of hits, shaped like `brok search` hits (`kind: "memory"`, the title as `symbol`). |
| `brok link list --json` | One array: `name`, `kind` (`local`, `git`), `root`, `path`, `git`, `branch`, `description`, `memory`, `index` (`own`, `managed`), `exists`, `files`, `chunks`, `error`. |
| `brok link add/remove/sync ... --json` | One document (`sync`: one result per link with `commit`, `files_indexed` or `error`). |
| `brok doctor --json` | One array of checks: `status` (`ok`, `warn`, `fail`), `name`, `detail`, `fix`. Exits 0 whatever the checks say. |
| `brok stats --json` | One document: `days`, `events`, `search` (`latency` {`count`, `p50_ms`, `p95_ms`}, `empty`, `semantic`, `with_links`, `with_memories`), `answers` (`latency`, `first_token`, `failed`, `models`, `skills`), `index` (`latency`, `files_indexed`, `last`), `errors` (`count`, `commands`, `recent[]`). |
| `brok bug --json --title ... [--what-happened ...] [--repo brok\|brok-agent\|ide] [--context ...]` | One document: `repo`, `url` (the prefilled issue form, to open in a browser), `file` (the saved report), `report` (Markdown, secrets masked). Opens nothing. |
| `brok config get [key] --json` | The configuration (or one key) as JSON; change it with `brok config set <key> <value>`. |
| `brok ask ... --json`, `brok review --json`, `brok skill run ... --json` | A stream, one JSON event per line (see below). |

Streaming events:

```json
{"type": "chunk", "text": "Refresh tokens rotate on every "}
{"type": "chunk", "text": "use..."}
{"type": "done", "provider": "ollama", "model": "qwen2.5-coder:7b",
 "sources": [{"path": "src/auth/auth.service.ts", "start_line": 42, "end_line": 77, "symbol": "refresh"}]}
```

A failure ends the stream with a non-zero exit code and the message on
stderr. Follow-up questions pass the conversation so far:
`brok ask --json --history turns.json "and where is it tested?"`, where
`turns.json` holds `[{"role": "user", "content": "..."}, {"role":
"assistant", "content": "..."}]`.

## Shards: integrations you build

When something is specific to your team (a ticketing system, a deploy
pipeline, a compliance check), write a shard instead of waiting for a core
feature: it reacts to workspace events and publishes its own. See
[shards.md](shards.md) and start with `brok shard new my-integration`.
