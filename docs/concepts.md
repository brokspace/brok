# Concepts

How Brokspace thinks about repositories, workspaces and integrations, and what exists today. Each capability carries a status: **available** (shipped and tested), **planned** (designed or tracked in an issue, not in the product). Nothing marked planned is promised for a date.

## Meta Workspace

A Meta Workspace coordinates several independent repositories as one engineering environment, for example a frontend, the APIs it calls and a shared library. You create it with `brok init` in the directory that coordinates them and register the repositories as **members** ([how](meta-workspace.md)). **Available** (opt-in; without members nothing changes).

| | Meta Workspace | Monorepo | Git superproject (submodules) |
|---|---|---|---|
| Repositories | Several, independent | One | Several, pinned by the parent |
| History | Each keeps its own | One shared | Each its own; the parent records commit pointers |
| Source control owner | Your existing tools | Your existing tools | Git |
| What Brokspace adds | Search, memory and agents across them, read only | Search, memory and agents in it | Same as a monorepo for what is checked out |
| Writes inside a repository | Never | Only what you ask | Only what you ask |

A Meta Workspace does not replace Git, your editor, your CI or your coding agents. It is a layer that lets them see the same set of repositories.

Boundaries: members are never merged into one repository, never indexed into one shared index, and never written to by registration or search. Removing a member unregisters it and leaves the directory alone.

## Repositories and identity

A member has a stable lowercase kebab-case **id**, a location (a local `path`, or a `git` URL that brok clones read only into `.brokspace/members/<id>/`) and an optional description. Ids are not reused: results, relationships and shard messages refer to them. **Available.**

**Relationships** are typed edges between members (`consumes_api`, `uses_library`, `depends_on` and the others in [Meta Workspace](meta-workspace.md#configuration)). Today they are recorded, validated and listed. Using them to steer search or agent work is **planned**.

## Local-first storage and isolation

- Each member keeps its own `.brokspace/` and index. A workspace search reads those indexes read only; it does not start, write or reindex them. Members without their own Brokspace are skipped with a note.
- Git URL members live under the coordinating workspace's `.brokspace/members/<id>/`, never in your own checkouts.
- Configuration is not merged across repositories: each workspace reads only its own `brok.yml`.
- Nothing leaves your machine unless you configured an LLM or remote embeddings that do. `brok.yml` holds no secrets; providers use credential references (an environment variable name) only.
- Shards run under brok's permission model and event bus ([architecture](architecture.md#event-bus)); the sandbox is advisory, as described there.

## Search scope

`brok search` stays on the current repository. `--scope workspace` (CLI), `"scope": "workspace"` (MCP `brok_search`) and `SearchScope::Workspace` (shards, `brok-sdk` 0.3) add the members, rank hits by score relative to each index's best, and prefix results with the member id. An agent or shard only crosses repositories when it asks to. **Available.**

## Providers

"Provider" means one of three integration categories, each with its own contract ([ADR 0015](adr/0015-provider-registry-and-source-control-contract.md)). They are unrelated to LLM providers (`llm.providers`) and to the Hub's distribution providers.

| Category | Responsibility | Status |
|---|---|---|
| **Source control** | Detect repositories, changed paths, history, nested repositories | **Available**: Git is the built-in default and behaves exactly as before. A registry and a capability contract exist in `brok-core`; Git is the only adapter. Other systems are **planned** and tracked in brokspace/brok#110 |
| **Engineering context** | Bring decisions, sessions and design context from other tools | **Planned**. Only designed. No integration, including Entire, is implemented, and none is a source-control provider |
| **Agent execution** | Run work through an external agent runtime under permission boundaries | **Planned**. Today agents reach Brokspace through MCP, skills and the `brok-agent` shard, which is not this category |

A source-control provider declares capabilities (`status`, `history`, `submodules`, later `diff`, `watch`, `worktrees`). An operation it does not support fails with an explicit `Unsupported` error; it never returns an empty success, so a partial provider is visible instead of silently wrong.

## Adopting it step by step

1. Keep working in one repository: nothing to change. Git is used as before.
2. `brok link` another repository for read only context ([linked workspaces](memory.md#linked-workspaces)); it keeps working unchanged.
3. Move to members when you want scoped search, status and git URL members: `brok workspace add`. Linked workspaces and members can coexist.
4. Remove a member at any time; no repository is modified. An older brok that meets `workspace.members` stops with an unknown-field error instead of ignoring them, so upgrade the whole team before committing members.

## Planned, not available

For clarity, these are tracked but **not** in the product: engineering-context and agent-execution providers; non-Git source control; using relationships in retrieval; collaboration modes; the development lifecycle workflow (`/work`); `brok verify` validation profiles. Follow brokspace/brok#108 and the issues it links.

## Writing a provider

The source-control contract and how to add and test an adapter are in [architecture](architecture.md#source-control-providers). Start from the Git adapter's tests: repository detection, nested repositories, a repository without commits and the `Unsupported` paths.

## When a provider or member is unavailable

| Symptom | Cause | What to do |
|---|---|---|
| A member is skipped in a workspace search | It has no `.brokspace` of its own, or its index cannot be read | `brok workspace status`; run `brok init` there and index it |
| A git URL member shows "not cloned" | Registered with `--no-fetch`, or the clone failed | `brok workspace sync <id>`; private repositories need SSH keys or a credential helper |
| A provider operation reports `Unsupported` | The provider lacks that capability | Not an error in your setup; use another operation or provider |
