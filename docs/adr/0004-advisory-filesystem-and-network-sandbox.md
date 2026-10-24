# 0004. Shard file system and network permissions are advisory for now

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

Manifests declare file system, network, process and environment capabilities. Enforcing file system and network limits needs a different OS mechanism on each platform (Landlock and seccomp on Linux, Seatbelt on macOS, AppContainer or job objects on Windows), each with its own constraints.

## Decision

For the first releases:

- **Enforced**: topic permissions on the bus, a scrubbed environment (OS basics plus the variables the manifest lists), the working directory, and per-process tokens.
- **Declared, shown for consent and exported to the shard** (`BROK_FS_READ`, `BROK_FS_WRITE`, `BROK_NETWORK`), but **not enforced by the OS**: file system and network access.
- `brok shard install` always shows the requested capabilities (and any build command it will run) and asks for consent.

## Consequences

- Users must only install shards they trust; the docs and `SECURITY.md` say so plainly.
- OS-level enforcement can be added per platform later without changing the manifest format, which is why the fields exist now.
- This ADR is superseded once OS sandboxes land.
