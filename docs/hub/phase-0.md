# Shard Hub, Phase 0: inventory, boundaries and threat model

Companion of the ADRs [0008](../adr/0008-supabase-as-hub-backend.md) to [0013](../adr/0013-open-core-and-repository-boundaries.md). Tracking issue: brokspace/brok#79; epic: brokspace/brok#101. Facts below were read from the repositories on 2026-11-22.

## 1. Inventory

| Piece | Where | What exists | Gap for the Hub |
|---|---|---|---|
| Manifest | `brok-sdk` `manifest.rs`, `shard.schema.json` | `shard.json` with strict validation (`deny_unknown_fields`), permissions, topics, `releases` with SHA-256 | No source coordinates, tree hash, attestation or MCP connector fields |
| Install | `brok` `crates/brok-shard` `install.rs` | Install from git (build) or checksum-pinned `releases`; skill-only shards | No provider abstraction, no digest verification against external evidence |
| Lockfile | `crates/brok-shard` `registry.rs` | `shards.lock` v1: version, source (git or local), binary SHA-256, install time | Unreadable lock is treated as empty (brokspace/brok#116); no artifact digest, no evidence |
| Supervisor | `crates/brok-shard` `supervisor.rs`, `sandbox.rs` | Spawns and restarts shards; filesystem and network permissions are advisory ([0004](../adr/0004-advisory-filesystem-and-network-sandbox.md)) | No health or resource metrics; no honest enforced/not-enforced status |
| Event bus | `crates/brok-bus` | Local bus with token auth and topic ACLs ([0003](../adr/0003-local-event-bus-with-capability-acls.md)) | Fine as is |
| MCP | `brok mcp` ([0006](../adr/0006-mcp-stdio-server-for-coding-agents.md)) | Server exposing `brok_search`, `brok_status` | No MCP connectors as installable packages |
| Template | `brok-shard-template` | Native shard with `shard.json`, CI | No publish workflow |
| CI | each repo | Required checks and rulesets; release and publishing workflows | No checker, no attestation step |

Reusable as is: manifest validation, bus, supervisor, install pipeline, the publishing workflows. Everything on the Hub side (backend, checker, action, portal) is new.

## 2. Repository and visibility plan

Smallest set first ([0013](../adr/0013-open-core-and-repository-boundaries.md)):

| Repository | Visibility | When |
|---|---|---|
| `brok`, `brok-sdk`, `brok-agent`, `brok-shard-template`, `website`, `vscode`, `ide` | public (exist) | now |
| `shard-hub` (portal, migrations, RLS, Edge Functions) | public | with brokspace/brok#80 |
| `brok-shard-check` (core, ruleset, adapters, action) | public | with brokspace/brok#88; the Action stays a directory until it needs its own releases (#93) |
| SDKs for other languages, `brok-shard-spec` | public | when the spec outgrows `brok-sdk` |
| `brokspace-enterprise`, `brokspace-hub-ops`, `brokspace-internal-admin` | private | only when the first code exists |
| `brand` | private (exists) | unchanged |

## 3. License audit (for maintainer review, not legal advice)

Rust and publishing manifests all say `MIT OR Apache-2.0`, with `LICENSE-MIT` and `LICENSE-APACHE` present. Mismatches found; published terms are not changed here.

| Repository | Declared | Files | GitHub detects | Note |
|---|---|---|---|---|
| `brok`, `brok-sdk`, `brok-agent`, `brok-shard-template`, `website` | `MIT OR Apache-2.0` (website: no manifest field) | both license files | `apache-2.0` | GitHub shows one license for a dual-licensed repository; expected. `website` lacks a declared license field |
| `vscode` | `MIT OR Apache-2.0` | `LICENSE` (states either of the two), plus both files | `other` | A custom `LICENSE` file hides the dual license from GitHub; consider removing it in favor of the two standard files |
| `ide` | `MIT` | `LICENSE` (MIT, also Eclipse Theia copyright) | `mit` | A different license from the rest of the org; inherited from the Theia base. Decide whether this is intended |
| `brand` | `SEE LICENSE IN LICENSE.md` | `LICENSE.md` | n/a (private) | Name and logo policy should be published separately from the code license |

Maintainer decisions needed: the `vscode` license file, the `ide` license, a published name and logo policy, and DCO versus CLA before accepting outside contributions to any commercial module.

## 4. Threat model as tracked controls

| Threat | Control | Issue |
|---|---|---|
| Package substitution | Immutable versions, digest, signature, CLI verification | #84, #86, #94 |
| Publisher impersonation | OAuth, GitHub App, namespace ownership | #81, #82 |
| Compromised repository | Commit and tag verification, provenance | #82, #94 |
| Malicious build scripts | Never built in the Hub backend; isolated CI without secrets | #88, #93 |
| Dependency confusion | Explicit namespace, pinned provider, lockfile | #81, #84, #86 |
| Typosquatting | Namespace reservation, name similarity checks, moderation | #81, #98 |
| SSRF through package URLs | Provider allowlist, redirect and DNS/IP validation | #84 |
| Archive traversal, zip bombs | Size, depth, expansion and path limits | #88, #89 |
| Secret leakage | Secret scanning, redaction, no secrets in untrusted jobs | #88, #93 |
| Prompt injection in skills | Pattern scanning and warnings | #90 |
| Tenant leakage in Supabase | RLS, isolation tests, server-only service role | #80 |
| Webhook spoofing and replay | HMAC-SHA256, idempotent delivery ids, authorization | #82 |
| Malicious telemetry | Local first, opt-in export, redaction | #96 |
| Compromised checker workflow | Pinned versions, OIDC identity allowlist, attestation verification | #93, #94 |
| Vulnerable checker toolchain | Pinned tools and actions, SBOM, controlled updates | #92, #93 |
| Declared permissions mistaken for enforced ones | Honest status per permission | #97 |
| Lockfile loss or downgrade | Fail on unreadable or newer lock | #116 |
| Hub abuse and cost | Rate limits, quotas, retention | #99 |

## 5. Exit criterion

No change to `shard.json`, `shards.lock`, the bus protocol or the Hub schema merges without the compatibility behavior written in an ADR or in the PR, per [0012](../adr/0012-shard-manifest-and-lockfile-compatibility.md).
