# Contributing to Brokspace

Thanks for helping! This guide gets you from zero to a merged pull request.

Before you start, skim three short files: [SCOPE.md](SCOPE.md) (what belongs in
the core and what belongs in a shard), [GLOSSARY.md](GLOSSARY.md) (the words we
use) and [AGENTS.md](AGENTS.md) (conventions, also read by coding agents).

## How work flows

Brokspace develops itself with the same skills it ships. Every change follows
one path, whether a human or a coding agent does the work:

1. **Issue.** A bug or idea is filed with the issue forms. Every new issue gets
   `needs-triage` automatically.
2. **Triage.** A maintainer (often with `/triage`) judges it against
   [SCOPE.md](SCOPE.md) and moves it to one state: `needs-info`,
   `ready-for-agent`, `ready-for-human` or `wontfix`. Labels and their meaning:
   [docs/agents/triage-labels.md](docs/agents/triage-labels.md). Questions left
   unanswered for 14 days close automatically.
3. **Grill and spec.** For anything non-trivial, align first: `/grill-with-docs`
   interviews you until the design is clear and updates
   [GLOSSARY.md](GLOSSARY.md) and [docs/adr/](docs/adr/) as decisions land;
   `/to-spec` turns the result into a spec issue.
4. **Tickets.** `/to-tickets` breaks a spec into small, independently mergeable
   tickets with explicit blocking edges.
5. **Implement.** One ticket at a time: test first (`tdd`), then a two-axis
   review (`code-review`: project standards, and faithfulness to the spec).
6. **Pull request.** Use the PR template: evidence that it works, and a risk
   call (one-way or two-way door, blast radius). The title follows
   Conventional Commits. CI must be green on Linux, macOS and Windows, for both
   the default and the lean build.
7. **Release.** Merged changes ship in the next release; user-visible changes
   are recorded in [CHANGELOG.md](CHANGELOG.md) under *Unreleased*.

The skills used above are built into `brok`. Install them into your coding
agent (Claude Code, Codex, Cursor, GitHub Copilot, ...) with
`brok skill export`, or run `brok setup agents` to also register the Brokspace
MCP server. `brok skill list` shows every available skill, and `/ask-brok`
tells you which one fits your situation.

Rejected ideas are recorded in [.out-of-scope/](.out-of-scope/) so the same
discussion does not happen twice; decisions we made are in
[docs/adr/](docs/adr/).

## Set up the polyrepo

Brokspace is split into repositories that live **side by side**:

```text
brokspace/
├── .cargo/config.toml     ← written by bootstrap: use the local brok-sdk
├── brok/                  ← this repository
├── brok-sdk/
├── brok-agent/
└── brok-shard-template/
```

```sh
mkdir brokspace && cd brokspace
git clone https://github.com/brokspace/brok
./brok/scripts/bootstrap.sh          # Windows: .\brok\scripts\bootstrap.ps1
```

The bootstrap script clones the missing siblings and writes
`brokspace/.cargo/config.toml`, which patches the crates.io `brok-sdk`
dependency of every repository with your local checkout. Change the SDK and
the CLI, agent and template pick it up immediately. Remove that file to build
against the published SDK.

## Toolchain

- Rust **1.89+** (`rustup update stable`).
- Default features need a C compiler (tree-sitter), `protoc` with the protobuf
  includes (LanceDB) and network access at build time (ONNX Runtime).
  - Debian/Ubuntu: `apt install build-essential protobuf-compiler libprotobuf-dev`
  - macOS: `xcode-select --install && brew install protobuf`
  - Windows: Visual Studio Build Tools (C++) and `winget install protobuf`
- No native toolchain? `cargo test --no-default-features` builds and tests the
  pure-Rust core, or use the dev container:

```sh
docker build -t brok-dev brok/.devcontainer
docker run --rm -it -v "$PWD:/src" -w /src/brok brok-dev cargo test
```

VS Code users can simply "Reopen in Container".

## Everyday commands

```sh
cargo fmt                                  # format
cargo clippy --all-targets -- -D warnings  # lint (CI denies warnings)
cargo test                                 # all tests, default features
cargo test --no-default-features           # pure-Rust build
cargo run -p brok-cli -- -C /some/workspace status
```

Handy while developing shards: `brok run -v` in one terminal and
`brok bus tail` in another.

## Guidelines

- **Small, focused pull requests** with a clear description of the why.
- **Tests**: new behaviour comes with tests. Prefer real I/O in temp dirs over
  mocks (see `crates/brok-index/tests/pipeline.rs`, `crates/brok-bus/tests/bus.rs`).
- **Errors**: libraries return errors with context (`anyhow::Context`) and
  never print; only `brok-cli` writes to the terminal. Messages should tell
  the user what to do next.
- **No panics on user input**; `unwrap` only where an invariant guarantees it
  (and say why in an `expect` message).
- **Unsafe** code is denied by default; the one exception (memory-mapping
  immutable Arrow files) carries a `SAFETY:` comment.
- **Cross-platform**: Linux, macOS and Windows are first-class. CI runs all
  three; mind path separators (`/` in stored paths) and named pipes.
- **Docs**: public items have doc comments; user-visible changes update
  `README.md`, `docs/` and `CHANGELOG.md`.
- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat(index): …`, `fix(bus): …`) make the changelog easy to write.

## Protocol and manifest changes

`brok-sdk` is a public contract used by third-party shards:

- additive changes (new optional fields, new topics) are fine in minor releases;
- anything breaking bumps `PROTOCOL_VERSION` and the SDK's major version, and
  needs an ADR in [docs/adr/](docs/adr/) describing the migration first.

## Releasing (maintainers)

1. If `brok-sdk` changed: bump its version, tag `vX.Y.Z` in brok-sdk; its
   Release workflow publishes it to crates.io.
2. Update `CHANGELOG.md`, bump `version` in `Cargo.toml` (workspace and the
   internal dependency versions), push, wait for green CI, then tag
   `vX.Y.Z`. The Release workflow builds binaries for Linux, macOS and
   Windows, attaches them with `.sha256` files and publishes the release
   (the install scripts always fetch the latest one).
3. Publish the crates: `gh workflow run publish-crates.yml -f tag=vX.Y.Z`
   (vendors the built-in skills into `brok-skills`, then publishes every
   crate in dependency order).
4. For brok-agent, after its release: `gh workflow run publish-crate.yml
   -f tag=vX.Y.Z`, then pin the new binaries in its `shard.json`
   `releases` (the Release workflow prints the block in its summary).

All publishing uses the `CARGO_REGISTRY_TOKEN` organization secret.

## Code of conduct

Participation is governed by our [Code of Conduct](CODE_OF_CONDUCT.md).
