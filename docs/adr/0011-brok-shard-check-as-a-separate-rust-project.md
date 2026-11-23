# 0011. `brok-shard-check` as a separate Rust project reusing `brok-sdk`

- **Status**: Accepted
- **Date**: 2026-11-22

## Context

The same rules must give the same answer on a publisher's laptop, in CI, in the Hub and in the CLI, for shards written in several languages.

## Decision

`brok-shard-check` is one Rust core with a library and a CLI, reusing the manifest types and validation of `brok-sdk` rather than reimplementing them. Language adapters (Rust, TypeScript, Python first) feed it facts; they never decide pass or fail. Output is a stable result model, serialised as JSON and SARIF, with an SBOM, and a canonical tree-hash format with test vectors (brokspace/brok#88, #89, #91, #92). It is a public project: the baseline ruleset must be reproducible by anyone.

It starts as its own repository because its release cadence, supply-chain pinning and CI-action packaging differ from the runtime, and the Hub and CLI both consume it as a crate.

## Consequences

- Manifest rules live in one place (`brok-sdk`), so `shard.json`, the JSON schema and the checker cannot drift. Any manifest change keeps updating `manifest.rs`, `shard.schema.json` and the docs together, as `brok-sdk/AGENTS.md` already requires.
- The checker reads untrusted input: archive, path, size and time limits are part of the first version, and it never executes package code.
- Results are versioned by ruleset, so a new rule does not silently change old verdicts.
