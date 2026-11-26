# Architecture decision records

One file per decision, numbered, in the format described in [0000](0000-record-architecture-decisions.md). These records apply to every repository in the brokspace organization.

| ADR | Decision | Status |
|---|---|---|
| [0000](0000-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0001](0001-polyrepo-with-brok-sdk-as-contract.md) | Polyrepo with brok-sdk as the shared contract | Accepted |
| [0002](0002-native-backends-behind-cargo-features.md) | Native backends behind default-on cargo features | Accepted |
| [0003](0003-local-event-bus-with-capability-acls.md) | Local event bus with token auth and topic ACLs | Accepted |
| [0004](0004-advisory-filesystem-and-network-sandbox.md) | File system and network permissions are advisory for now | Accepted |
| [0005](0005-agent-skills-format-for-skills.md) | Agent Skills (SKILL.md) as the skill format | Accepted |
| [0006](0006-mcp-stdio-server-for-coding-agents.md) | MCP stdio server for coding agents | Accepted |
| [0007](0007-lancedb-remote-feature-workaround.md) | LanceDB `remote` feature workaround | Accepted (temporary) |
| [0008](0008-supabase-as-hub-backend.md) | Supabase as the Shard Hub backend | Accepted |
| [0009](0009-federated-distribution-and-distribution-provider.md) | Federated distribution behind a `DistributionProvider` | Accepted |
| [0010](0010-trust-model-and-attestations.md) | Trust model: independent signals and verifiable evidence | Accepted |
| [0011](0011-brok-shard-check-as-a-separate-rust-project.md) | `brok-shard-check` as a separate Rust project | Accepted |
| [0012](0012-shard-manifest-and-lockfile-compatibility.md) | Compatibility rules for `shard.json` and `shards.lock` | Accepted |
| [0013](0013-open-core-and-repository-boundaries.md) | Open core and repository boundaries | Accepted |
| [0014](0014-meta-workspace-extends-linked-workspaces.md) | Meta Workspace extends linked workspaces | Accepted |
| [0015](0015-provider-registry-and-source-control-contract.md) | Provider registry and a capability-based source-control contract | Accepted |
| [0016](0016-remote-obsidian-vaults-by-mirroring.md) | Remote Obsidian vaults: mirror them with a shard, keep core off the network | Accepted |
| [0017](0017-compact-vector-storage.md) | Store vectors as float16, keep 768 dimensions | Accepted |
| [0018](0018-text-first-indexing.md) | Index text first, then add vectors in saved batches | Proposed |
