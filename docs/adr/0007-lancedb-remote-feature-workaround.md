# 0007. Enable LanceDB's `remote` feature as a build workaround

- **Status**: Accepted (temporary)
- **Date**: 2026-10-24

## Context

`lancedb` 0.39 and 0.40 do not compile with default features: `src/job.rs` constructs `Error::Http`, a variant that only exists behind the `remote` feature, and the `job` module itself is not feature gated.

## Decision

The workspace declares `lancedb = { version = "0.40", features = ["remote"] }` with a comment pointing to this problem. Brokspace never connects to LanceDB Cloud; the feature only makes the crate compile.

## Consequences

- Extra dependencies (gRPC, OAuth, an HTTP client) are compiled into the default build, which increases build time and binary size.
- Drop the feature (and supersede this ADR) as soon as an upstream release gates `job.rs` correctly; dependabot groups LanceDB updates to make that check routine.
