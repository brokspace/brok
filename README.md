# Brokspace

**Your workspace, understood. Local-first search, agents and skills for every developer.**

Brokspace (`brok`) makes any development workspace, from a side project to a
corporate monorepo with dozens of submodules, instantly searchable and
agent-ready. One command sets everything up; nothing leaves your machine
unless you choose a hosted model.

```console
$ brok init
[1/6] Workspace        ✓ Created .brokspace
[2/6] Agent            ✓ Installed brok-agent 0.1.0
[3/6] Semantic search  ✓ Embedding model ready
[4/6] Local LLM        ✓ Found Ollama with 3 model(s): brok ask will use qwen2.5-coder:7b
[5/6] Index            ✓ Indexed 1 284 files (9 912 chunks)
[6/6] Coding agents    ✓ Claude Code: MCP server registered in .mcp.json, 20 skills in .claude/skills

$ brok search "where do we validate jwt tokens"
0.97 services/auth/src/jwt.rs:42-77 function validate_token
```

## What you get

- **Search that understands code.** Functions, classes and sections are
  indexed whole (tree-sitter), found by meaning or by name, across every
  git submodule. Works offline; works without any LLM.
- **Your coding agent, Brokspace-aware.** `brok setup agents` gives Claude
  Code, Cursor, VS Code / Copilot and Codex a `brok_search` tool (MCP) and a
  set of proven engineering **skills**.
- **A local LLM in one step.** `brok setup llm` sizes a model to your machine
  and wires up Ollama or LM Studio. Hosted models work too.
- **Shards: build your own integrations.** Small event-driven programs that
  react to your workspace (a change, a question, a review) with only the
  permissions they declare. Scaffold one with `brok shard new`.
- **Skills: share your team's workflows.** Write a `SKILL.md` once, use it in
  every agent and on your local LLM; publish a set as a skill shard.

Simple by design: zero required configuration, sensible defaults, one file
(`.brokspace/brok.yml`) when you want to change something.

## Install

**macOS / Linux**

```sh
curl -fsSL https://raw.githubusercontent.com/brokspace/brok/main/scripts/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/brokspace/brok/main/scripts/install.ps1 | iex
```

Managed (corporate) Windows machines often block downloaded scripts. Then
install by hand: download `brok-windows-x86_64.exe` and
`brok-agent-windows-x86_64.exe` from the
[latest release](https://github.com/brokspace/brok/releases/latest) and the
[agent release](https://github.com/brokspace/brok-agent/releases/latest),
check them against the `.sha256` files, rename them to `brok.exe` and
`brok-agent.exe`, and put both in one folder on your `PATH`.

Both install `brok` and `brok-agent` (checksums verified). Prebuilt
binaries: Linux x86_64/arm64, macOS Apple Silicon, Windows x86_64, and macOS
Intel (text search only: ONNX Runtime has no Intel Mac build). Prefer Cargo?
`cargo install brok-cli brok-agent` (needs a C compiler and `protoc`; or
`--no-default-features` for a pure-Rust, text-search-only build).

## Quick start

```sh
cd your-project              # one repo, or a folder holding many repos
brok init                    # guided setup, ~1 minute plus downloads
brok search "how does X work"
brok ask "how is a refresh token rotated?"   # starts brok in the background
```

Then, in your coding agent: `/ask-brok` to pick a workflow, `/grill-me`
before a big change, or just ask it to "use brok_search".

`brok doctor` checks everything and tells you how to fix what is missing.
New here? Follow the **[getting started guide](docs/getting-started.md)**.

## A day with brok

```sh
brok start                       # once a day (or let `brok ask` start it): live indexing in the background
brok chat                        # think through a change with follow-up questions
brok review                      # AI review of your uncommitted changes (or --staged, --base main)
git add -p && brok commit        # Conventional Commits message from the staged diff, then commit
brok skill run tdd "add retry to the http client"
brok stop                        # when you are done
```

The index follows you: edits, commits, pulls and branch switches are picked
up within seconds, and `brok start` catches up on anything that changed
while it was off. The embedding model and your LLM choice are shared by all
your workspaces, so `brok init` in the next project takes seconds.

## Everyday commands

| Command | What it does |
|---|---|
| `brok init [--yes]` | Guided setup: workspace, agent, search model, local LLM, index, coding agents |
| `brok search <query> [-n N] [--json]` | Hybrid search (semantic + keywords) |
| `brok ask <question> [-p provider]` | Answer from your LLM, grounded in your code, with sources |
| `brok chat [--skill name]` | Conversation with follow-ups (`/skill`, `/clear`, `/exit`) |
| `brok review [--staged \| --base main]` | Code review of your changes with the `code-review` skill |
| `brok commit [--yes \| --print]` | Commit message for the staged changes, then commit |
| `brok start` / `stop` / `restart` / `logs -f` | The background daemon (bus, live indexing, agent) |
| `brok status` / `brok doctor` | What is indexed and running / health check with fixes |
| `brok index [--rebuild]` | Re-index (incremental; the daemon does it automatically) |
| `brok setup llm \| agents \| sync` | Local LLM, coding agent integrations, index mirroring |
| `brok skill list \| show \| run \| new \| export` | Agent skills |
| `brok shard new \| install \| list \| info \| remove` | Shards |
| `brok mcp` | MCP server for coding agents (started by them) |
| `brok completions <shell>` | Shell completions (bash, zsh, fish, powershell) |
| `brok run` / `brok bus tail \| emit` | Foreground daemon / watch and inject events (shard development) |
| `brok model download \| status` | Embedding model |

## Guides

| | |
|---|---|
| **[Getting started](docs/getting-started.md)** | **Step by step: install, first workspace, daily use, configuration, troubleshooting** |
| [Local LLM setup](docs/local-llm.md) | Ollama / LM Studio, which model fits your machine |
| [Integrations](docs/integrations.md) | Coding agents (MCP + skills), LLMs, storage |
| [Skills](docs/skills.md) | Built-in workflows, writing and sharing skills |
| [Shards](docs/shards.md) | Build event-driven integrations, `shard.json` reference |
| [Configuration](docs/configuration.md) | Every `brok.yml` key |
| [Architecture](docs/architecture.md) | How indexing, search and the bus work |

## How it works

```text
            coding agents ──MCP──┐        ┌── brok ask / skill run / bus
                                 ▼        ▼
   ┌──────────────────────── brok run: local event bus ─────────────────────┐
   │  index service          brok-agent               your shards          │
   │  hybrid search          git change tracking      (sandboxed, with     │
   │  hot/cold partitions    review · RAG · skills     declared topics)    │
   └──────┬──────────────────────────┬───────────────────────────────────────┘
          ▼                          ▼
   .brokspace/databases       Ollama / LM Studio / hosted LLM
```

Code is chunked along its syntax tree, embedded locally
(`nomic-embed-text-v1.5` on ONNX Runtime) and stored in LanceDB or
memory-mapped Arrow files next to a BM25 index; search fuses both rankings.
Each git submodule is its own small *hot* database, so changes stay cheap and
searches favour the code you are working in. Details in
[docs/architecture.md](docs/architecture.md).

## Repositories

| Repository | Purpose |
|---|---|
| [`brok`](https://github.com/brokspace/brok) | This repo: CLI, event bus, indexer, shard runtime, built-in skills |
| [`brok-sdk`](https://github.com/brokspace/brok-sdk) | Protocol, `shard.json` schema and client library for shards |
| [`brok-agent`](https://github.com/brokspace/brok-agent) | The default shard: change tracking, review, answers, skills |
| [`brok-shard-template`](https://github.com/brokspace/brok-shard-template) | Template for your own shard |

## Security

The bus is local-only and token-authenticated; every shard is limited to the
event topics its manifest declares, starts with a scrubbed environment, and
its requested permissions are shown before install. File system and network
permissions are not yet enforced by the OS (on the roadmap), so install
shards you trust. See [SECURITY.md](SECURITY.md).

## Contributing

Brokspace is built with the workflow it ships: issues are triaged with a
small label state machine, changes go through spec, tickets, test-first
implementation and review. Start with [CONTRIBUTING.md](CONTRIBUTING.md),
[SCOPE.md](SCOPE.md) and [GLOSSARY.md](GLOSSARY.md); questions go to
[Discussions](https://github.com/brokspace/brok/discussions).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
