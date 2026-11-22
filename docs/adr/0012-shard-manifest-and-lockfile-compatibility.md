# 0012. Compatibility rules for `shard.json` and `shards.lock`

- **Status**: Accepted
- **Date**: 2026-11-22

## Context

The Hub adds data (source coordinates, digests, attestations) to files that already exist in users' workspaces. `shards.lock` is a versioned JSON file (`version: 1` today) and `shard.json` rejects unknown fields.

## Decision

1. **No breaking change to either format without compatibility behavior** written down first: what an old `brok` does with a new file and what a new one does with an old file.
2. `shard.json` additions are optional fields, added to `brok-sdk` first, released, then used by the Hub. A new `brok` reads every manifest an older one wrote. Because unknown fields are rejected by design, an older `brok` refuses a manifest that uses a new field with a clear message; publishers who need to support both ranges keep new fields out until the range of `brok` they declare knows them.
3. `shards.lock` stays `version: 1` while changes are additive. Hub-installed entries add optional fields (provider, coordinate, artifact digest, tree hash, evidence digest). A reader ignores optional fields it does not know; a change that removes or reinterprets a field bumps the version, and `brok` migrates a lock forward on write, keeping a backup.
4. **Prerequisite**: today `read_lock` treats a lock it cannot parse as empty and the next install overwrites it. Before any lock change ships, an unreadable or newer-version lock must stop the command with an error and leave the file untouched (brokspace/brok#116).
5. Existing git-source and `releases` installs keep working unchanged.

## Compatibility matrix

| Writer \ Reader | Old `brok` | New `brok` |
|---|---|---|
| Old manifest / lock v1 | works | works |
| New manifest with optional fields | refused with a readable error (unknown field) | works |
| New lock v1 with optional fields | works, extra fields dropped on its next write | works |
| Lock v2 | **today: read as empty and overwritten** (see 4); after the fix, refused with "upgrade brok" | works, reads v1 and v2 |

## Consequences

- Tests pin both directions (fixtures of old files read by new code).
- The lock never stores secrets, tokens or personal data.
