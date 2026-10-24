# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
