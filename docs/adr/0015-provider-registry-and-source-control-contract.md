# 0015. Provider registry and a capability-based source-control contract

- **Status**: Proposed
- **Date**: 2026-11-22

## Context

brokspace/brok#109 asks for source control behind a contract, with Git as the default adapter. Today Git is reached by shelling out in two places: `crates/brok-core/src/git.rs` (`run`, `is_repository`, `submodules`, `nested_repositories`, `lfs_track`) and the change tracking of `brok-agent` (`src/tracker.rs`). "Provider" here means source control, engineering context and agent execution; it is unrelated to the Hub's distribution providers ([0009](0009-federated-distribution-and-distribution-provider.md)).

## Decision

1. **Where it lives.** The registry and the Rust trait start in `brok-core`. Wire-level types move to `brok-sdk` only when an out-of-process shard or `brok-agent` needs them, which is a versioned public contract change and gets its own ADR. Until then the agent keeps its own Git calls and is aligned in a follow-up.
2. **Three categories, three contracts**: `SourceControl`, `EngineeringContext`, `AgentExecution`. Only `SourceControl` is designed now.
3. **Registry.** Providers register under a unique id with a contract version and a capability set; registering a duplicate id is an error with a clear message. `brok doctor` lists providers and their capabilities.
4. **Capabilities, not universal semantics.** A provider declares which of these it supports: `status`, `diff`, `history`, `watch`, `submodules`, `worktrees`, `lfs`. Any operation the provider does not support returns an explicit `Unsupported { capability }` result, never an empty success. Mutations (branch, commit, stash, worktree) are not part of the required contract.
5. **Minimal required operations**, derived from what the code uses today: `open(path)` and identity, `status` / changed paths, `nested repositories`, and a reconcile entry point that returns the changed paths since an opaque revision token. Revision ids are opaque strings, kept as the provider returns them.
6. **Git adapter.** Wraps the existing functions without changing arguments, environment or output handling, so behavior is identical; `git.rs` becomes the adapter's implementation. The git command used stays the one on `PATH`.
7. **No secrets.** Provider configuration holds credential **references** (an environment variable name or keychain label), never values; logs and the index redact them.

## Alternatives considered

- **A trait covering all of Git**: loses providers with fewer concepts and bakes in Git's model.
- **Generic lowest-common-denominator API**: hides Git features the product needs. Capabilities avoid this.
- **Start in `brok-sdk`**: freezes an unproven API as a public contract before a second provider exists.

## Consequences

- Phase 1 of #109 is a refactor with a test that pins current Git behavior (including nested repositories, submodules and a repository without commits) before moving code.
- A second adapter is not built in #109. The contract is validated by a minimal in-memory test provider that supports few capabilities, so the `Unsupported` paths are exercised.
- Names of external projects (Entire, Scion) are not part of this decision; supporting them needs their API identified first (brokspace/brok#110).
