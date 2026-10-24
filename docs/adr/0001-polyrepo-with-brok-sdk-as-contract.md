# 0001. Polyrepo with brok-sdk as the shared contract

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

Brokspace has parts with different audiences and release cadences: the runtime and CLI, the protocol used by third-party shards, the default agent, and a template for shard authors. Shard authors must be able to depend on the protocol without pulling in the runtime.

## Decision

- Each part is its own repository under the `brokspace` organization: `brok` (Cargo workspace with core, bus, index, shard and CLI crates), `brok-sdk`, `brok-agent`, `brok-shard-template`, plus the org-wide `.github`.
- `brok-sdk` is the only shared contract. Every other repository depends on it from crates.io (`brok-sdk = "0.1"`), never through a committed path dependency, so any repository builds when cloned alone.
- For local development the repositories sit side by side and `brok/scripts/bootstrap.(sh|ps1)` writes an untracked `../.cargo/config.toml` with `[patch.crates-io] brok-sdk = { path = "brok-sdk" }`. CI recreates the same layout.

## Consequences

- Shards (including ones installed from source by `brok shard install`) build against the published SDK.
- Changing the SDK and a consumer at the same time needs coordinated pull requests; the patch makes that seamless locally.
- `brok-sdk` must be published to crates.io before the other crates can be published, and protocol changes follow the compatibility rules in its `AGENTS.md`.
