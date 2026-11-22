# 0010. Trust model: independent signals and verifiable evidence

- **Status**: Proposed
- **Date**: 2026-11-22

## Context

A single "verified" badge hides what was actually checked. Users and policies need to know which claims have evidence.

## Decision

Trust is a set of independent signals, each with its own evidence: `Publisher Verified`, `Repository Verified`, `Signed Artifact`, `Provenance Verified`, `Scanned`, `Hub Policy Passed`, `Official`, plus the states `Unverified`, `Quarantined` and `Blocked`. A package page shows the checker and ruleset version, scan date, analysed digest and per-control results, and its limitations. `Scanned` never implies "no vulnerabilities".

Evidence is **verifiable, not claimed**: signatures and provenance come from GitHub Artifact Attestations and/or Sigstore, verified against a pinned OIDC issuer and workflow identity. A report supplied by a publisher is never proof without an attestation. The CLI repeats the checks locally.

Installation policy: `Official` requires signature, provenance and policy pass; community packages require a valid manifest, an immutable digest and the mandatory analysis; `Unverified` installs only after an explicit warning and a local policy that allows it; `Blocked` is refused by default; a permission expansion on upgrade asks for consent again.

## Consequences

- The badge vocabulary and policy are part of the public contract (brokspace/brok#94).
- Declared permissions are not enforced permissions; the UI and CLI must say which ([0004](0004-advisory-filesystem-and-network-sandbox.md), brokspace/brok#97).
- Anti-abuse heuristics may stay private, but they can never be the only basis of a public policy result ([0013](0013-open-core-and-repository-boundaries.md)).
