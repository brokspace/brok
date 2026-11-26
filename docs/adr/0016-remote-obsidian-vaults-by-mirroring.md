# 0016. Remote Obsidian vaults: mirror them with a shard, keep core off the network

- **Status**: Proposed
- **Date**: 2026-11-26

## Context

brokspace/brok#104 asks for a vault that is not on this machine, through the community [Local REST API](https://github.com/coddingtonbear/obsidian-local-rest-api) plugin. Today a vault is a folder: memories are written to `memory.obsidian.folder`, and `memory.obsidian.search: vault` reads every note, within a size budget.

What the plugin offers, from its README and OpenAPI description (read 2026-11-26):

- HTTPS on port 27124 with `Authorization: Bearer <key>`. Plain HTTP on 27123 is off by default.
- It creates its own certificate authority. The server certificate is only valid for `127.0.0.1`, `localhost`, the configured binding host and any extra names the user adds in the plugin.
- `GET /vault/` lists a directory and `GET /vault/{path}` returns a note. Search endpoints exist (`/search/simple/`, JsonLogic on `/search/`). It also writes, deletes and runs Obsidian commands.
- Wrong keys are throttled (10 a minute, then 429).

Not verified: whether directory listing is recursive (assume it is not), the name and default of the binding setting, and size limits (none found).

The API key is powerful: it can change and delete notes and run commands, and the plugin offers no read-only key as far as we found.

## Decision

1. **A separate integration shard (`brok-obsidian`) mirrors the remote vault into a local, read-only cache.** Core then searches that folder with the existing vault search. Core gets no new network code, no API key and no new ranking path, and a vault that is offline stays searchable from the last copy.
2. **One direction, remote to local, in the first version.** The shard sends only `GET` requests. Saving memories to a remote vault is out of scope; it would need conflict handling and a second decision.
3. **Explicit sync.** On demand and on an optional interval. Only notes that are new or changed are fetched, deleted notes are removed from the mirror, and an interrupted run leaves the previous mirror usable (write to a temporary file, then rename).

```yaml
shards:
  brok-obsidian:
    settings:
      url: https://127.0.0.1:27124
      api_key_env: OBSIDIAN_API_KEY      # the name of a variable, never the key
      ca_file: ~/.config/brokspace/obsidian-ca.crt
      exclude: [Private]
```

## Threat model

| Risk | Rule |
|---|---|
| Interception | HTTPS only, verified against `ca_file` (the plugin CA). There is no option to skip verification. |
| Request forgery to other hosts | Requests go only to the configured host and port. Redirects are refused. |
| Key leakage | Read from the environment variable named in settings, listed in the shard's `permissions.env`. Never written to `brok.yml`, the lockfile, logs, errors or the mirror. |
| Misuse of a powerful key | The client implements `GET` only, covered by a test against a fake server that fails on any other method. |
| Hostile file names | Paths from the server are validated before they touch the disk: relative, no `..`, no absolute or drive paths, no hidden folders, no symlinks followed, and unique ignoring case. |
| Oversized or endless vaults | Limits on directory depth, number of notes, bytes per note and in total; timeouts on every request; the run reports what it left out. |
| Secrets inside notes | The mirror is a copy of the user's notes on this machine, inside `.brokspace/` (not committed). Search results keep masking secrets. `exclude` is applied before a note is downloaded. |
| Offline or wrong key | A clear message, the previous mirror stays, no retry loop that triggers the throttle. |

Shard sandboxing is advisory ([0004](0004-advisory-filesystem-and-network-sandbox.md)): these rules are enforced by the shard's own code and tests, not by the operating system. For a vault on another machine, we recommend a private tunnel (SSH or a VPN such as Tailscale) over exposing the plugin to a network.

## Alternatives considered

- **A live backend in core**, querying the plugin at search time: puts network, credentials and a third-party API in core, makes every search depend on the other machine, and mixes the plugin's ranking with ours. Rejected.
- **A shard that answers searches itself:** shards cannot contribute results to `brok search` today; it would need a new SDK contract.
- **Only synced folders** (the current answer: Obsidian Sync, Syncthing, git, iCloud): still the recommendation when it works. The shard is for those who cannot or will not copy the whole vault by those means.

## Consequences

- A new repository `brokspace/brok-obsidian`, a good first integration package for the Hub (brokspace/brok#101).
- Core needs one small change: the mirror folder must be usable as `memory.obsidian.vault` with writing turned off, so brok never saves memories into a copy that the next sync overwrites.
- Tests before release: contract tests against a fake server (listing, changes, deletions, hostile names, limits, timeouts, bad certificate, wrong key), and a manual check against the real plugin, which also settles the points marked not verified.

## Questions for the maintainer

1. Is read-only (mirror) enough for the first version?
2. May a new `brokspace/brok-obsidian` repository be created from the shard template?
3. Can you test against a real vault with the plugin before it is released?
