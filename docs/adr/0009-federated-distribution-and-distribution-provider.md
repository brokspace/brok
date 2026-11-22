# 0009. Federated distribution behind a `DistributionProvider`

- **Status**: Proposed
- **Date**: 2026-11-22

## Context

Shards are published by third parties already using npm, crates.io, PyPI, Maven Central, Go modules and GitHub Releases. Hosting artifacts ourselves adds storage, egress and abuse costs, and a registry to defend, without helping the first users.

## Decision

The Hub is an **index and a trust-evidence service**, not an artifact host. Artifacts stay in the publisher's registry. Each registry is reached through one provider contract: resolve a coordinate to a version, return an immutable artifact descriptor (URL, size, digest), download with safe limits, and verify. The MVP ships GitHub Releases and npm; crates.io, PyPI, Maven and Go follow (brokspace/brok#95). The CLI verifies the digest before installing, and never trusts the Hub's word alone.

## Alternatives considered

- **Own OCI registry**: strongest control, highest cost. Deferred until a measurable requirement (availability, takedown, air-gap mirrors) justifies it.
- **Git-only installs** (today's `brok shard install <url>`): remains supported as the escape hatch, but gives no discovery or evidence.

## Consequences

- Not hosting artifacts does not remove the duty to verify source, digest, license, permissions and provenance.
- A version is immutable: a different digest under the same version is rejected.
- Provider downloads are an SSRF and archive-bomb surface: allowlisted hosts, redirect and DNS/IP checks, size and expansion limits (see the threat model in [the Phase 0 report](../hub/phase-0.md)).
- Each new provider is code behind the contract, with a conformance test suite; no provider-specific branches in the catalog.
