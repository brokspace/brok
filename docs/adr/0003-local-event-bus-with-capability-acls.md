# 0003. Local event bus with token auth and manifest-enforced topic ACLs

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

Shards are third-party processes. They need a fast way to exchange events with the runtime and each other, without being able to impersonate one another, read other shards' private replies or drive the runtime itself.

## Decision

- `brok run` hosts a local bus on a Unix domain socket (mode 0600) or a Windows named pipe that rejects remote clients. Frames are length-prefixed JSON with a 16 MiB cap.
- Every connection presents a 256-bit random session token. Tokens are issued per process (one per shard launch, one operator token for the CLI) and stored hashed.
- The token resolves to a grant derived from the shard manifest: publishing outside `publishes_to` or subscribing outside `subscribes_to` is rejected with `Forbidden`.
- The bus overwrites each event's `source` with the authenticated identity.
- The `system:` namespace is reserved for the runtime and broadcast to everyone.
- Replies (events with a `correlation_id`) are delivered only to the client that published the request.

## Consequences

- A shard cannot spoof another shard or eavesdrop on request/reply traffic, and permission problems surface as explicit errors.
- A single router actor owns routing state; slow consumers lose events (and the loss is logged) instead of stalling the bus.
- The protocol is language neutral: any process that can open the socket or pipe and speak the frames can be a shard.
