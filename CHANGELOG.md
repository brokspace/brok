# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Machines without a GPU get a model one size smaller (`brok init`,
  `brok setup llm`): `qwen2.5-coder:3b` with 16 GB, `qwen2.5-coder:7b` with
  32 GB. On a laptop CPU a 7B model needs 3 to 5 minutes per review; 3B is
  about twice as fast. With an NVIDIA GPU or Apple Silicon nothing changes.

## [0.2.2] - 2026-10-30

### Added

- `brok upgrade`: shows what changed since your version (from this
  changelog), downloads the new `brok` and `brok-agent`, verifies their
  checksums, swaps them in (also on Windows, where a running program cannot
  be overwritten) and restarts brok in every workspace where it was running.
  `brok upgrade --check` only reports; `--to <version>` picks a version.
  Installs made with Cargo are pointed to `cargo install` instead.
- `brok doctor` says when a newer brok is available.
- Each workspace's copy of `brok-agent` is refreshed when brok starts and the
  agent next to `brok` is newer, so upgrades reach every workspace.

### Fixed

- `brok stop` (and `brok restart`) return only after the daemon process has
  exited, not when it stops listening: replacing `brok.exe` right after a
  stop (an upgrade) no longer fails with "file in use" on Windows.

## [0.2.1] - 2026-10-30

### Changed

- Lock files (`pnpm-lock.yaml`, `package-lock.json`, `npm-shrinkwrap.json`,
  `go.sum`, plus `*.lock` as before) are not indexed by default: in a small
  NestJS project `pnpm-lock.yaml` alone was 64% of the chunks and the top
  search hit, and indexing took 5x longer.
- JavaScript and TypeScript functions written as `const name = () => ...`
  (or wrapped, like `memo(() => ...)`) are indexed as functions with their
  name, instead of anonymous preamble code.
- `brok review` sends the diff first and a self-contained instruction last,
  so local models review instead of asking for a spec.
- Answers wait up to 10 minutes for the first word, with a spinner: a 7B
  model on a laptop CPU needs minutes to read a review prompt. The 3-minute
  limit still applies between words.
- `brok bus tail` shows the time of day instead of epoch milliseconds.

### Fixed

- Claude Code (VS Code / JetBrains extension) and Cursor are detected from
  their settings folder in the home directory, so `brok init` and
  `brok setup agents` set them up without a `claude` command on `PATH`.
- Saving `brok.yml` (`brok setup llm`, `brok setup sync`, `brok init`) keeps
  the explanatory comments of the template.
- `brok mcp` ignores a byte order mark at the start of a message.
- Hints point to `brok start` / `brok restart` instead of the foreground
  `brok run` (`brok doctor`, `brok shard install`, `brok shard new`).

## [0.2.0] - 2026-10-25

### Added

- `brok start`, `stop`, `restart` and `logs`: the daemon runs in the
  background; `ask`, `chat`, `review`, `commit` and `skill run` start it on
  demand and wait until the agent is ready.
- `brok chat`: conversations with follow-up questions, `/skill`, `/clear`.
- `brok review` (uncommitted, `--staged` or `--base <ref>`) and
  `brok commit` (Conventional Commits message from the staged diff, then
  commit), plus the built-in `commit-message` skill.
- Workspaces made of several independent git repositories: nested
  repositories become hot partitions like submodules.
- The daemon catches up on changes made while it was stopped.
- `brok completions <shell>`.
- User-level defaults: the embedding model is cached once per user and the
  LLM chosen with `brok setup llm` applies to every new workspace.

### Fixed

- Piping `brok ask` no longer hangs on Windows when it starts the daemon.

## [0.1.1] - 2026-10-24

### Fixed

- Windows: `brok.exe` declares long path awareness, so LanceDB writes work in
  deep workspaces when long paths are enabled; `brok doctor` explains how to
  enable them.

### Documentation

- Manual install steps for Windows machines whose antivirus blocks the
  install script.

## [0.1.0] - 2026-10-24

### Added

- `brok init`, `index`, `search`, `ask`, `run`, `status`, `setup`, `shard`,
  `model` and `bus` commands.
- AST chunking with tree-sitter (Rust, Python, JavaScript, TypeScript/TSX,
  Go, Java); heading-aware Markdown and block chunking for other text.
- Apache Arrow chunk schema with LanceDB and memory-mapped Arrow IPC stores.
- Code-aware BM25 and hybrid search with Reciprocal Rank Fusion.
- Local `nomic-embed-text-v1.5` embeddings on ONNX Runtime (CPU, CUDA,
  CoreML, DirectML).
- Hot (submodule) / cold (root) partitions with federated search.
- Local event bus over Unix sockets / named pipes with token
  authentication, manifest-based topic ACLs and request/reply routing.
- Shard registry, installer (git or local, checksum-verified releases),
  scrubbed environments and a restarting supervisor.
- Sync target configuration for S3, Cloudflare R2, MinIO and Git LFS.
- Guided `brok init` (workspace, agent, search model, local LLM, first index,
  coding agents) and `brok doctor` with fix suggestions.
- `brok setup llm`: detects Ollama and LM Studio, recommends a model sized to
  the machine's memory and can download it; `brok setup agents` and
  `brok setup sync`.
- Agent skills: 19 built-in skills (engineering and productivity buckets),
  `brok skill list|show|run|new|export`, workspace overrides in
  `.brokspace/skills/`, and skill shards (`"skills"` in `shard.json`,
  `entrypoint` now optional).
- `brok mcp`: Model Context Protocol server with `brok_search` and
  `brok_status` for Claude Code, Cursor, VS Code / Copilot and Codex.
- `brok shard new` (from the template, or `--skills-only`).
- One-line installers for macOS/Linux and Windows.

### Project

- Development lifecycle adopted from [mattpocock/skills](https://github.com/mattpocock/skills):
  `AGENTS.md` (with `CLAUDE.md` pointing to it), `GLOSSARY.md`, `SCOPE.md`,
  `.out-of-scope/` decision records and architecture decision records in
  `docs/adr/`.
- Issue triage state machine (`needs-triage`, `needs-info`, `ready-for-agent`,
  `ready-for-human`, `wontfix`) documented in `docs/agents/`, with workflows
  that label new issues and close unanswered `needs-info` issues after 14 days.
- Bug and idea issue forms, a pull request template with a risk section, and
  `CODEOWNERS`.
- "How work flows" section in `CONTRIBUTING.md`.
