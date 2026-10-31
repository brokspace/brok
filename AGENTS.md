# AGENTS.md

Operating guide for coding agents and contributors working in `brokspace/brok`: the `brok` CLI, the event bus, the indexer, the shard runtime and the built-in skills.

Brokspace is a local-first productivity platform for personal and corporate workspaces. Keep it **simple and zero-config**: domain-specific behaviour belongs in **shards** (standalone event-driven mini-agents) and **integrations** (coding agents through skills and MCP, local LLMs, git, storage), not in the core. Read [SCOPE.md](SCOPE.md) before proposing new core features.

## Layout

```text
crates/
  brok-core/    workspace discovery, .brokspace/ layout, brok.yml, partitions, git helpers
  brok-bus/     local event bus: listener, sessions, router actor, token auth and topic ACLs
  brok-index/   chunking (tree-sitter), Arrow schema, BM25, stores, embeddings, hybrid search
  brok-shard/   shard registry, installer, sandbox environment, supervisor
  brok-cli/     the `brok` binary, one module per subcommand (src/cmd/)
skills/         built-in Agent Skills (SKILL.md), embedded in the binary
  engineering/  daily code work
  productivity/ daily non-code workflow
docs/           user and contributor docs; docs/adr/ holds decisions, docs/agents/ holds agent config
scripts/        bootstrap (polyrepo dev setup) and install scripts
.out-of-scope/  rejected ideas and why (read before proposing)
```

Shared vocabulary lives in [GLOSSARY.md](GLOSSARY.md): use its terms in code, docs, issues and commit messages (for example "partition", never "shard of the index").

## Build and test

Brokspace is a polyrepo. `brok-sdk` is consumed from crates.io; for local development the repositories sit side by side and a parent `.cargo/config.toml` patches `brok-sdk` to the sibling checkout:

```sh
./scripts/bootstrap.sh            # Windows: .\scripts\bootstrap.ps1 (clones siblings, writes ../.cargo/config.toml)
cargo test --no-default-features  # pure-Rust build: text chunking, Arrow store, BM25. Always works.
cargo test                        # default features: tree-sitter, LanceDB, ONNX Runtime (needs a C compiler + protoc)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

No native toolchain? Use the dev container: `docker build -t brok-dev .devcontainer` then run cargo inside it with this repository and its siblings mounted (see [CONTRIBUTING.md](CONTRIBUTING.md)). LanceDB needs about 6 GB of RAM to compile; use `-j 4` on small machines.

Before calling work done, run fmt, clippy (`-D warnings`) and tests for **both** feature sets when you touched `brok-index` or feature-gated code.

## Conventions

- Libraries return errors with context (`anyhow::Context`, `thiserror` for typed errors) and **never print**. Only `brok-cli` talks to the terminal (`crates/brok-cli/src/ui.rs`). Error messages tell the user what to do next.
- No panics on user input. `unwrap`/`expect` only where an invariant guarantees it, and say why in the `expect` message.
- `unsafe` is denied workspace-wide. The single exception (memory-mapping immutable Arrow files) carries a `SAFETY:` comment.
- Cross-platform is first class (Linux, macOS, Windows). Stored paths are workspace relative with `/` separators. Mind named pipes, file locks and antivirus locks on Windows (`fsutil::move_dir`).
- Public items have doc comments. Module docs explain the why.
- Tests exercise behaviour through public interfaces with real I/O in temp dirs (see `crates/brok-index/tests/pipeline.rs`, `crates/brok-bus/tests/bus.rs`). No mocks of our own internals.
- New dependencies: prefer pure Rust; anything needing a C toolchain goes behind a cargo feature so the lean build keeps working.
- Protocol and manifest types live in `brok-sdk` (a separate repository and a public contract). Additive changes only, unless an ADR covers the migration.
- Commit messages and PR titles follow Conventional Commits with the crate as scope: `feat(index): ...`, `fix(bus): ...`.
- No em-dashes in prose (docs, skills, changelog, comments). Rewrite the sentence with a comma, colon or period instead.

## Git workflow

Every change reaches `main` through `staging`, the default branch:

```text
your branch --PR, squash--> staging --release PR, merge commit--> main --> tag
```

1. Branch from staging: `git fetch origin && git switch -c <type>/<topic> origin/staging` (for example `feat/kotlin-chunker`, `fix/empty-config`).
2. Commit in small steps with Conventional Commits. End every commit message with a `Co-Authored-By:` trailer naming the agent, for example `Co-Authored-By: Claude <noreply@anthropic.com>`: the PR gets the `ai-agent` label from it.
3. Before pushing, run the fmt, clippy and test commands from "Build and test" above.
4. Push, open the PR against staging with the template filled in, and turn on auto-merge:

   ```sh
   git push -u origin HEAD
   gh pr create --base staging --fill
   gh pr merge --auto --squash
   ```

   The PR merges itself once `ubuntu-latest (default)`, `ubuntu-latest (lean)`, `macos-latest (default)`, `macos-latest (lean)`, `windows-latest (default)`, `windows-latest (lean)`, `MSRV (1.89, lean)` and `policy / title` pass and review conversations are resolved. No approval is needed.
5. A check failed? Read it (`gh pr checks --watch`, `gh run view --log-failed`), fix it on the same branch and push; auto-merge stays on. Cannot fix it? Say what you tried in a PR comment and stop. Never weaken or skip a check to get green.

Never push to `staging` or `main` (both reject it), force-push a branch someone else pushed to, open or merge PRs into `main`, or merge with `--admin`. The release PR into `main` is opened by the Promote workflow and merged by a maintainer.

Do not commit a `Cargo.lock` change that only adds `[[patch.unused]]`: it comes from the local `../.cargo/config.toml` patch, and CI rejects it. Restore the file (`git restore Cargo.lock`) before committing.

## Skills

Built-in skills under `skills/` use the Agent Skills format (`<name>/SKILL.md` with `name` and `description` frontmatter, plus `agents/openai.yaml`). A skill is either **user-invoked** (`disable-model-invocation: true`, the human types `/name`) or **model-invoked** (the agent may reach for it on its own). Whenever you add, rename or remove a skill:

- keep `skills/README.md` and the bucket `README.md` (grouped into User-invoked and Model-invoked) in sync;
- keep the `ask-brok` router skill accurate;
- make sure `brok skill list` shows it (skills are embedded at build time).

Install the skills into your own coding agent with `brok skill export` (see `brok setup agents`).

## Agent skills configuration

- **Issue tracker**: GitHub Issues on [brokspace/brok](https://github.com/brokspace/brok/issues), driven with `gh`. See [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).
- **Triage labels**: canonical role names map to label strings in [docs/agents/triage-labels.md](docs/agents/triage-labels.md). Judge every issue against [SCOPE.md](SCOPE.md); rejected enhancements get a file in [.out-of-scope/](.out-of-scope/).
- **Domain docs**: glossary in [GLOSSARY.md](GLOSSARY.md), decisions in [docs/adr/](docs/adr/) (one file per decision, `NNNN-kebab-title.md`).
