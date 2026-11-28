# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `brok status --json` reports `readiness`: whether keyword and meaning search, the agent and the model are ready, with one suggested next action and a stable code for it, so editors need not read error text. Only a model server on this machine is contacted (brokspace/brok#201).

### Fixed

- `brok search` no longer says to enable embeddings when they are enabled and search by meaning is not ready yet.
- `brok index` finishing an interrupted run says how many chunks got their vector, instead of only "Indexed 0 files (0 chunks)".
- `brok index --rebuild`, the fix `brok doctor` suggests for an index it cannot read, now works on one: it failed while opening the damaged data.
- `brok status --json` reports an index it cannot read, with `brok index --rebuild` as the next step, instead of failing (brokspace/brok#201).

## [0.12.0] - 2026-11-27

### Changed

- Search works within seconds on a new workspace: indexing first stores the text and keyword index, then computes vectors, saving about once a minute. Interrupting keeps what is done and the next run continues. A partition is searched by meaning once 95% of it has vectors, and `brok status` and `brok doctor` show the progress. The daemon answers searches while embedding and takes over from a `brok index` running beside it. On Linux, memory freed between the passes is handed back to the system, so the peak stays close to a single pass. New and rebuilt indexes work this way; brok 0.11 and earlier rebuild them (brokspace/brok#200, ADR 0018).

### Fixed

- A search on an Arrow index could fail with "No such file or directory" when the index was written at that moment; it now reads the newer data.
- A command run in the moment a daemon is starting waits for it instead of failing with "cannot read .../operator.token", or with an invalid token left behind by a daemon that crashed.

## [0.11.0] - 2026-11-27

### Added

- Shards in Python, TypeScript and Java: a guide to the protocol and packaging (`docs/shards-other-languages.md`) and three working examples in `examples/shards`, tested in CI on Linux and macOS.
- `brok setup agents --dry-run [--json]` lists the files connecting your coding agents would write.

### Changed

- New indexes store vectors as `float16`: about 35% (Arrow) to 49% (LanceDB) less disk for the chunk store, with the same search results in our measurements. Existing indexes keep working and are not rebuilt; `brok index --rebuild` converts one. brok 0.10 and earlier rebuild an index made this way (brokspace/brok#73, ADR 0017).

### Fixed

- `brok review` in a repository without commits reviews the staged files, or says to `git add` them, instead of showing git's error; outside a repository it says so (brokspace/brok#199).
- `ask`, `chat`, `review`, `commit` and `skill run` stop at once, saying what to do, when no shard can answer, instead of waiting for up to 10 minutes. `brok start` no longer fails after a minute when the agent cannot connect: it starts and warns (brokspace/brok#197).
- **Security:** private files under `.brokspace/` (private memories including saved chats, metrics, member and linked repository clones, shard data and bug reports) could be committed to git. New workspaces ignore them and existing ones are repaired by the next command, keeping your own rules; `brok doctor` reports files already tracked. Ignoring does not remove them from git history: see "Private files were committed" in the troubleshooting guide (brokspace/brok#195).
- Semantic search in large workspaces (10 000 chunks or more, LanceDB store) now uses its vector index: it was built with one metric and searched with another, so every search scanned all chunks. In our measurements vector search at 100 000 chunks takes about 9 ms instead of 75, returning 98.9% of the exact results. An existing index is replaced the next time indexing changes something (brokspace/brok#192).
- `brok init` and the daemon refuse a `brok-agent` that says it needs another brok version, keeping the installed one, and `brok doctor` reports a mismatch with `brok upgrade` as the fix (brokspace/brok#46).
- `brok init` without a terminal and without `--yes` no longer writes agent files into your project; the interactive question lists them (brokspace/brok#198).

## [0.10.0] - 2026-11-26

### Added

- `brok doctor` warns when `embeddings.batch_size` is large enough to take gigabytes while indexing on a CPU, and shows how to lower it; it changes nothing itself (brokspace/brok#72).
- `brok storage [--json]` reports the disk a workspace uses per kind of data and per partition, the shared model cache, and leftovers (indexes of removed repositories, replaced chunk store generations, old temporary files). It deletes nothing (brokspace/brok#74).

### Changed

- Built on `brok-sdk` 0.3.1: events waiting to be handled are limited to 32 MiB as well as 256 events, so large events can no longer queue gigabytes (brokspace/brok-sdk#25).
- Semantic indexing uses far less memory on a CPU: `embeddings.batch_size` now defaults to 1 (about 0.5 GB instead of 3.1 GB at peak, and faster). Existing `brok.yml` files that set 16 keep it; see the troubleshooting guide (brokspace/brok#72).
- Faster text search for common words: only the results that are returned get sorted, with the same ranking and scores (brokspace/brok#162).
- Indexing no longer keeps a list of every workspace file in memory while it looks for changes (brokspace/brok#72).

### Fixed

- `brok start` returns once the index service can answer, and commands that reach a daemon that is still starting wait for it, instead of timing out after 60 seconds (brokspace/brok#179).

## [0.9.0] - 2026-11-26

### Added

- A step-by-step guide, `docs/shard-tutorial.md`, builds a complete shard from an empty folder; the shard reference lists the shards made by Brokspace, and its examples and the `write-shard` skill declare a `brok` range that includes current releases.
- `memory.obsidian.read_only` searches a vault brok must never write to, such as the mirror of a remote vault (brokspace/brok#104).
- Obsidian notes for a better graph: `memory.obsidian.names: title` names new notes after their title (the id stays in the note, so renaming the file never breaks updates), notes link related memories as `[[wikilinks]]` and list the files they mention, properties you add are kept, and `brok memory export --obsidian` copies your private and shared memories into the vault (brokspace/brok#103).

## [0.8.0] - 2026-11-26

### Added

- `memory.obsidian.search: vault` searches every Markdown note of your Obsidian vault, read only, alongside your memories: hidden folders and `memory.obsidian.exclude` are skipped, `tags` and `aliases` are searchable, secrets are masked in results, and a size budget keeps large vaults within bounds. Off by default; brok still writes only to the memory folder (brokspace/brok#102).

### Documentation

- New Concepts page: Meta Workspace compared with a monorepo and a Git superproject, repository identity, local-first isolation, provider categories with their real status (available or planned), and step by step adoption.

## [0.7.0] - 2026-11-24

### Added

- The index service answers `SearchRequest` with `scope: workspace` by searching the Meta Workspace members too (`member:path` hits), so shards and agents can search across repositories. Requires `brok-sdk` 0.3.
- Shards learn the Meta Workspace members through `client.context().members` (id, directory, whether the member has a Brokspace of its own), so an agent can search the repositories around its workspace. Requires `brok-sdk` 0.2.

## [0.6.0] - 2026-11-23

### Added

- Meta Workspace foundation: `brok workspace add|list|status|remove|relate|unrelate|discover`
  registers independent repositories as members of a workspace and records
  typed relationships between them (`workspace.members` and
  `workspace.relationships` in `brok.yml`). Opt-in: workspaces without members
  are unchanged. See `docs/meta-workspace.md` (brokspace/brok#108).
- `brok search --scope repository|workspace`: `workspace` also searches the
  members that have their own Brokspace, read-only, and names the member of
  each hit (`member` in `--json`). The default is unchanged.
- The MCP tool `brok_search` accepts `scope: "workspace"` to search member
  repositories, and `brok status` (and `--json`, as `members`) lists them. Both
  only appear or change when a workspace has members.
- `brok doctor` lists the source-control provider with its contract version and
  capabilities, and checks each Meta Workspace member (brokspace/brok#109,
  brokspace/brok#108).
- Meta Workspace members can be git URLs: `brok workspace add <git-url>`
  (`--branch`, `--no-fetch`) clones the repository read only into
  `.brokspace/members/<id>/` and indexes it; `brok workspace sync` pulls and
  re-indexes; `remove` deletes what brok made. URLs with credentials are
  refused (brokspace/brok#108).

### Fixed

- `brok search | head` and other piped commands no longer panic when the reader closes the pipe; brok exits quietly like `grep` does (brokspace/brok#115).

## [0.5.0] - 2026-11-23

### Added

- Source control sits behind a capability-based `SourceControl` contract with
  a provider registry in `brok-core`; Git is the default adapter and behaves
  as before (brokspace/brok#109, ADR 0015).
- Release builds attach Debian packages (`brok_<version>_amd64.deb`, `arm64`)
  next to the standalone binaries, and the new manual workflows publish a
  signed APT repository and update the `brokspace/homebrew-brok` tap. See
  `docs/publishing.md`.
- Obsidian as an optional memory store and second brain: set
  `memory.obsidian.vault` (a local or cloud-synced vault folder) and use
  `--scope obsidian` in `brok memory`, or `scope: "obsidian"` in the MCP
  tools. Notes live in `<vault>/Brokspace/`, are searched with your code and
  are masked for secrets before they are written. A vault that does not exist
  is an error, never created. See `docs/obsidian.md`.

### Changed

- crates.io publishing uses Trusted Publishing (OIDC) instead of a
  `CARGO_REGISTRY_TOKEN` secret, and starts on the release tag but waits for
  the GitHub release to be published.
- The Linux ARM64 build now checks the glibc floor (2.39) like x86_64 does.

## [0.4.1] - 2026-11-14

### Fixed

- An unreadable `shards.lock`, or one written by a newer brok, is no longer
  read as empty and overwritten: installs and removals stop with an error and
  leave the file untouched (`brok shard list` still works). A missing lock is
  still an empty one (#116).
- Saving brok.yml (`brok config set`, `brok link`, editor settings) keeps
  your comments, key order and line endings, and adds new keys in their
  section. A file that cannot be merged safely is written as before.
- The Linux x86_64 binary ran only on the newest distributions: it was built
  on `ubuntu-latest` and needed its glibc (2.43) and libssl 3. It is now built
  on Ubuntu 20.04 with OpenSSL and libstdc++ linked in, so it runs on glibc
  2.31 and newer (Ubuntu 20.04+, Debian 11+). The release fails if it needs
  more. See `scripts/build-linux-compat.sh`.

### Added

- Cargo feature `vendored-openssl` on `brok-cli`: builds OpenSSL into the
  binary instead of using the system libssl.

## [0.4.0] - 2026-11-03

### Added

- Local metrics: searches, answers, indexing runs and failed commands are
  recorded in `.brokspace/metrics/` (timings and counts only, never queries or
  content; nothing leaves the machine). `brok stats [--days N] [--json]` shows
  latencies (p50, p95), failures and the latest errors. `metrics.enabled` and
  `metrics.retention_days` in brok.yml.
- `brok bug`: a report with versions, OS, `brok doctor`, metrics, the end of
  the log and brok.yml (secrets masked, home directory as `~`), saved to
  `.brokspace/reports/`, and a prefilled issue form opened in the browser.
  `--repo brok|brok-agent|ide`, `--json` for editors.
- `brok doctor --json`, and doctor checks linked workspaces.
- docs/troubleshooting.md: common problems, metrics and bug reports.

## [0.3.0] - 2026-11-02

### Added

- Memory: decisions, facts, notes, handoffs and chat sessions kept between
  sessions as Markdown files, private (`.brokspace/memory/`), shared with the
  team (`.brokspace-memory/`, committed) or global (every workspace). Search
  includes them; `brok memory add | list | search | show | forget`; MCP tools
  `brok_memory_save`, `brok_memory_search` and `brok_memory_recent`. Secrets
  are masked before anything is saved. See docs/memory.md.
- Linked workspaces (`workspace.links`): the services and libraries a
  workspace depends on, as a path or a git URL, searched with it (results like
  `../checkout/src/orders.ts`, labeled `@checkout`). `brok link add | list |
  sync | remove`; git links are cloned and indexed in `.brokspace/links/`.
- `brok config get | set | unset`: read and change `brok.yml` with validation.
- `brok status --json` reports `links` and `memory`.

### Changed

- Search ranks every partition, linked workspace and memory together, with
  a text relevance that compares across indexes (the share of the query's
  terms a chunk contains, relative to the best match in its index). A strong
  match in another submodule or a linked workspace now beats a weak one in
  the current partition; before, each partition's best hit scored the same.

## [0.2.4] - 2026-11-01

### Added

- JSON output for editors and scripts: `brok status --json` (workspace,
  daemon, index, LLM and shards in one document; `{"initialized": false}`
  outside a workspace) and `--json` on `brok ask`, `brok review` and
  `brok skill run`, streaming one event per line (`chunk`, then `done` with
  the sources). See docs/integrations.md.
- `brok ask --history <file>`: follow-up questions with the conversation so
  far, for chat interfaces built on the CLI.

### Fixed

- "Starting brok in the background" goes to stderr, so `brok ask ... > file`
  and JSON readers get only the answer on stdout.

## [0.2.3] - 2026-10-30

### Changed

- Machines without a GPU get a model one size smaller (`brok init`,
  `brok setup llm`): `qwen2.5-coder:3b` with 16 GB, `qwen2.5-coder:7b` with
  32 GB. On a laptop CPU a 7B model needs 3 to 5 minutes per review; 3B is
  about twice as fast. With an NVIDIA GPU or Apple Silicon nothing changes.

### Fixed

- A shard whose `shard.json` was saved with a byte order mark (Notepad,
  Windows PowerShell 5.1) is no longer ignored as malformed (brok-sdk 0.1.2).

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
