# Memory and linked workspaces

Brokspace remembers what matters between sessions, and knows about the
other workspaces yours depends on. Both reach every way you use brok:
`brok search` and `brok ask`, your editor, and your coding agents over MCP.

## Memories

A memory is a short Markdown note: a decision and why it was taken, a fact
about the code or the system, where work stopped. Brokspace searches them
with your code, so an answer next week starts from what you settled today.

```sh
brok memory add "Profiles are cached for 5 minutes" --kind decision --tag cache \
  --body "loadProfile goes through Redis; invalidate on profile updates."
brok memory list
brok memory search "cache"
brok memory show <id>
brok memory forget <id>
```

Kinds: `note` (default), `decision`, `fact`, `session` and `handoff`.

### Where memories live

| Scope | Folder | Who sees it |
|---|---|---|
| `private` (default) | `.brokspace/memory/` | You, in this workspace. Never committed. |
| `shared` | `.brokspace-memory/` at the workspace root | Everyone who clones the repository: commit the folder. |
| `global` | `memory/` in your user configuration directory | You, in every workspace (personal preferences). |

Pick the scope with `--scope`. The user configuration directory is
`~/.config/brokspace` on Linux, `~/Library/Application Support/brokspace`
on macOS and `%APPDATA%\brokspace` on Windows (`BROK_CONFIG_DIR`
overrides it).

Memories are plain files with a YAML front matter, so you can also write,
edit and review them by hand:

```markdown
---
title: Orders are idempotent per cart id
kind: fact
tags: [checkout]
created: 2026-11-02T20:14:03Z
updated: 2026-11-02T20:14:03Z
---

createOrder can be retried safely: it returns the existing order for a cart.
```

A Markdown file without front matter works too: its first heading becomes
the title.

### How memories are captured

- **By you**, with `brok memory add` or in your editor.
- **By your coding agents**: `brok mcp` offers `brok_memory_save`,
  `brok_memory_search` and `brok_memory_recent`, and tells agents to catch up
  at the start of a task and to save decisions and handoffs.
- **By your editor**: each chat with the Brokspace agent is saved as a `session`
  memory, so later questions can build on it. Turn this off with
  `memory.sessions: false`.

Anything that looks like a secret (API keys, tokens, private keys,
passwords in `KEY=value` form, credentials in URLs) is replaced with
`[secret]` before a memory is written. Forgetting a private or global
memory moves it to `.forgotten/` next to it, so nothing is lost by mistake;
a shared memory is deleted (git keeps its history).

## Linked workspaces

Services and libraries rarely live alone. When you work on the `user`
service, Brokspace should know the `checkout` service it calls. Link it:

```sh
brok link add checkout ../checkout \
  --description "Carts and payments; the user service calls its createOrder API"
brok link add payments https://github.com/acme/payments.git --branch main
brok link list
brok link sync            # pull git links, re-index what brok manages
brok link remove payments
```

Links are kept in `brok.yml`, so you can also write them by hand (and
commit them with a shared `brok.yml`):

```yaml
workspace:
  links:
    checkout:
      path: ../checkout
      description: Carts and payments; the user service calls its createOrder API.
    payments:
      git: https://github.com/acme/payments.git
      branch: main
      memory: false   # do not search its memories
```

What brok does with each kind:

| Link | Index | Updates |
|---|---|---|
| `path` to a Brokspace workspace | Its own index and memories, read only | Its own `brok run` (or `brok link sync`). |
| `path` to any other directory | Indexed by this workspace in `.brokspace/links/<name>/state/` | Every `brok start`, and `brok link sync`. |
| `git` URL | Cloned (shallow) into `.brokspace/links/<name>/src/` and indexed there | `brok link sync` (fetch, then incremental index). |

Search results from a linked workspace keep paths you can open from your
workspace (`../checkout/src/orders.ts`, `.brokspace/links/payments/src/...`)
and are labeled with the link (`@checkout`). They are weighted by
`search.link_weight` (0.8), so your own code wins ties. A link's
description is searchable too: questions about the dependency find what it
is and how you use it. Linked workspaces' shared memories (and private ones,
when the link is a Brokspace workspace on your machine) are searched unless
`memory: false`.

Links do not chain: the links of a linked workspace are not followed.
`brok link add` restarts the background brok when it runs, so searches see
the new link at once.

## Configuration

```yaml
memory:
  enabled: true    # search memories and let tools save them
  sessions: true   # save editor chat sessions as memories
  global: true     # search your global memories in this workspace
search:
  link_weight: 0.8
```

See [configuration.md](configuration.md) for every key, and
[integrations.md](integrations.md) for the JSON output editors use.
