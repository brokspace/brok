# Meta Workspace

A Meta Workspace coordinates several **independent repositories** as one engineering environment: a frontend, the APIs it calls, the shared library. It is not a monorepo and not a Git superproject. Each repository keeps its own history, configuration and index; Brokspace never writes inside one.

It is **opt-in**. A workspace with no members works exactly as before, and `brok.yml` gains nothing until you add one. Design: [ADR 0014](adr/0014-meta-workspace-extends-linked-workspaces.md). This page describes what exists today; workspace-wide workflows beyond search are planned (brokspace/brok#108).

```sh
brok init                                  # in the directory that coordinates the repositories
brok workspace discover                    # repositories next to it that are not members yet
brok workspace add ../payment-api -d "Payments service"
brok workspace add ../frontend
brok workspace relate frontend consumes_api payment-api
brok workspace status
```

| Command | What it does |
|---|---|
| `add <path> [--id] [--name] [-d]` | Registers a directory. Repeating it is a no-op. The id defaults to the directory name as kebab-case. |
| `list [--json]` | Members and relationships. |
| `status [--json]` | Per member: repository, plain directory or missing; whether it has its own Brokspace; commit and uncommitted changes (through the Git provider). |
| `remove <id>` | Unregisters a member and its relationships. The directory is never touched. |
| `relate <from> <kind> <to>` / `unrelate` | Records or removes a typed relationship. |
| `discover [dir]` | Lists repositories directly inside `dir` (default: the parent of the workspace) that are not members. |

## Configuration

```yaml
workspace:
  members:
    payment-api:
      path: ../payment-api        # relative to this workspace, stored that way so brok.yml can be committed
      description: Payments service
    frontend:
      path: ../frontend
  relationships:
    - from: frontend
      kind: consumes_api
      to: payment-api
```

- A member is **either** a `path` (a directory on this machine) **or** a `git` URL (with an optional `branch`), never both.
- A member id is lowercase kebab-case, stable, and not reused. `name` is an optional display name.
- Relationship kinds: `depends_on`, `consumes_api`, `publishes_api`, `uses_library`, `deploys_with`, `tests`, `shares_contract_with`. Both ends must be members, an edge cannot point at itself, and duplicates are rejected.
- Precedence, most specific first: the repository's own `brok.yml`, then this workspace's, then your user configuration, then defaults.
- An older brok that meets `members` stops with its usual unknown-field error rather than ignoring them.

## Git URL members

```sh
brok workspace add https://github.com/acme/payment-api.git            # clone, index, register
brok workspace add git@github.com:acme/payment-api.git --branch main --id payments
brok workspace add <url> --no-fetch                                    # register now, clone later
brok workspace sync [id...]                                            # pull and re-index
```

```yaml
workspace:
  members:
    payment-api:
      git: https://github.com/acme/payment-api.git
      branch: main
```

brok clones the repository shallow and read only into `.brokspace/members/<id>/src/` and keeps its index in `.brokspace/members/<id>/state/`, so `--scope workspace` searches it like any member. Your own repositories are never touched. What brok clones follows your Git configuration: use SSH keys or a credential helper for private repositories.

- **No credentials in `brok.yml`.** It is committed, so a URL with a password or token (`https://user:token@host/...`) is refused, and the error never repeats it. `git@host:org/repo` and `ssh://git@host/...` are fine: `git` there is a username.
- Registering the same URL again (however it is spelled: trailing `/`, `.git`, case) is a no-op. A different branch of the same repository needs its own id.
- A clone is not a workspace of its own: if the repository commits a `.brokspace/`, brok ignores it and keeps its own index.
- `brok workspace remove <id>` deletes the clone and index brok made, and nothing else.
- Until `brok workspace sync` has run, `brok status` and `brok doctor` show the member as not cloned and a workspace search skips it with a note.

## Rules registration enforces

- The workspace itself, `.brokspace` and a missing directory are refused.
- A nested repository or submodule **inside** the workspace is already indexed as a partition, so it is refused instead of being indexed twice.
- Members cannot contain each other, so indexes never overlap.
- An id used by another directory, or a directory already registered under another id, is refused.

## Relationship to linked workspaces

[Linked workspaces](memory.md#linked-workspaces) (`brok link`) keep working unchanged: read-only extra context that does not chain. Members are the opt-in, richer model, and are searched with `--scope workspace` (below).

## Failure behavior and security

Registration edits only `brok.yml`, atomically; an interrupted run leaves the previous file. Paths are canonicalized, so two spellings of one directory are the same member. Nothing is sent anywhere, and a member's contents are not read by these commands except for `git status` and the current commit.

## Scoped search

```sh
brok search "invoice total"                      # this repository only (default, unchanged)
brok search "invoice total" --scope workspace    # this repository and every member
```

`--scope workspace` searches each member that has its own Brokspace (its own `.brokspace`, created with `brok init` there) through its existing index, read-only: a member is never indexed, written to or started by a search. Results name the member (`api:src/billing.rs`, and a `member` field in `--json`). Members without a Brokspace, or whose index cannot be read, are listed as skipped (`skipped_members` in `--json`) and the search continues.

Each index scores on its own scale, so hits are ranked by their score relative to the best hit of their own index; on a tie this repository comes first. Semantic and text search follow each member's own `embeddings` setting.

### From your coding agents

The MCP tool `brok_search` takes the same choice as an optional argument: `"scope": "workspace"` searches the members too, and each result is prefixed `member:path`. Without it the tool behaves as before, so an agent only crosses repositories when it asks to.

## Status

`brok status` lists the members under "Meta Workspace members" (missing, with or without a Brokspace of its own), and `brok status --json` gains a `members` array with `id`, `root`, `state` and `has_brokspace`. The key is absent in a workspace without members, so the output of every other workspace is unchanged.
