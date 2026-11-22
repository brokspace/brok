# 0014. Meta Workspace: extend linked workspaces, do not add a second model

- **Status**: Proposed
- **Date**: 2026-11-22

## Context

brokspace/brok#108 asks for a Meta Workspace: several independent repositories coordinated as one engineering environment, without a monorepo or a superproject. Brok already has two relevant pieces:

- **Linked workspaces** (`workspace.links` in `brok.yml`, `brok link add|sync`, `crates/brok-core/src/links.rs`): another directory or a git repository cloned by brok is searched read-only, weighted by `search.link_weight`. Links deliberately do not chain: a link of a link is not followed.
- **Partitions** for submodules and nested repositories (`crates/brok-core/src/git.rs`), indexed as hot and cold partitions under `.brokspace/databases/`.

`brok.yml` is versioned and rejects unknown fields, and everything brok keeps lives under `.brokspace/`.

## Decision

A Meta Workspace is **a workspace whose `brok.yml` lists member repositories**, not a new kind of directory or a new store.

1. **Members are the generalisation of links.** A member has a stable workspace-local `id` (kebab-case, never reused), a `path` or `git` source (as links), a display name and optional declared capabilities. The existing `workspace.links` keeps working unchanged and reads as members that declare no relationships; no migration is required, and `brok link` stays.
2. **Opt-in.** Without a `workspace.members` section nothing changes: one repository, one workspace, today's commands. The new section and the commands of #108 (`brok workspace ...`) are behind it.
3. **One index per repository.** Member identity namespaces the existing per-workspace databases and state; indexes are never merged into one. The meta layer holds only the member list and a small relationship graph, so it stays light regardless of repository size.
4. **Relationships are data, not behavior.** Typed edges (`depends_on`, `consumes_api`, `publishes_api`, `uses_library`, `deploys_with`, `tests`, `shares_contract_with`) between member ids, stored in `brok.yml` next to the member and read by search, status and agents. They never trigger writes in a member.
5. **Scope is explicit.** Searches take `--scope repository|workspace`; the default stays the current one. Cross-repository results always name the member.
6. **Discovery and registration are idempotent.** Registering a path already known (by canonical path, or by git common dir for worktrees and submodules) is a no-op, nested repositories and submodules are registered once, and traversal tracks canonical paths to stop loops. Registration writes `brok.yml` through the same atomic write the rest of brok uses, so an interrupted run leaves the old file.
7. **Configuration precedence**, most specific wins: repository `brok.yml` > meta workspace `brok.yml` > user config > defaults. A member's own configuration is read-only input to the meta workspace.
8. **Git stays the authority** for branches, commits and history of every member; the meta workspace stores none of it.

## Alternatives considered

- **A new `.brokspace/meta.yml` manifest**: a second configuration file with its own precedence rules and a split-brain risk with `brok.yml`. Rejected.
- **A Git superproject or monorepo import**: violates "independent repositories" and changes source-control ownership.
- **One merged index**: unbounded memory and rebuild time, and it loses per-repository status.

## Consequences

- The links model is extended deliberately: links keep their no-chaining rule, and members are the only thing that can be registered into a meta workspace; a member that is itself a meta workspace is not expanded (depth one), to keep results and cycles predictable.
- `brok.yml` gains `workspace.members` and `workspace.relationships`, with the `version` rule applying: an older brok that meets them fails with its usual unknown-field error rather than ignoring them. Document this and gate the new keys behind the release that knows them ([0012](0012-shard-manifest-and-lockfile-compatibility.md) applies the same reasoning to shards).
- Tests required by #108: one repository, many, nested, mixed; idempotent re-registration; interrupted write; and regression runs of the existing single-repository commands.
- The agent (brokspace/brok-agent#22) watches the repositories of a workspace; reaching it needs only the member list through the existing shard context, so no SDK change is needed to start.
