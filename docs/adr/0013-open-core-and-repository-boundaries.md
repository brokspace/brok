# 0013. Open core and repository boundaries

- **Status**: Proposed
- **Date**: 2026-11-22

## Context

Trust requires that the rules behind "Hub Policy Passed" can be read and reproduced. Commercial sustainability requires a place for organization-specific features.

## Decision

Public: `brok`, `brok-sdk`, the `shard.json` schema, templates, `brok-shard-check` and its baseline ruleset, the CI action, the Hub public API and client, and the Hub frontend, migrations, RLS policies and Edge Functions. Private: enterprise modules (SSO/SCIM, organization policy, audit export, private registries and mirrors, air-gapped deployment), internal operations tooling, production topology, and anything holding secrets or personal data. Secrets are never in Git, in any repository.

Enterprise modules consume **stable public contracts** (the SDK, the provider contract, the policy and result schemas) and never fork the core. A contract change that an enterprise module needs is made in the public repository first.

Repositories are created when ownership, cadence or lifecycle demands it, smallest set first: start `shard-hub` and `brok-shard-check` as public repositories (different release cadences, own CI and secrets); keep the GitHub Action as a directory of `brok-shard-check` until it needs its own versioned releases; create the private repositories only when the first enterprise or operations code exists.

Licensing stays `MIT OR Apache-2.0` for Rust code and for the new public repositories unless a maintainer and legal review decide otherwise. The Brokspace name and logo are governed by a separate policy, so a fork may use the code but not impersonate an official publisher.

This is technical guidance, not legal advice.

## Consequences

- See the license audit and the repository plan in [the Phase 0 report](../hub/phase-0.md).
- Contributor terms (DCO or CLA) need a maintainer decision before outside contributions to commercial modules.
