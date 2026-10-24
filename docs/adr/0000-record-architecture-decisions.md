# 0000. Record architecture decisions

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

Brokspace spans several repositories and is built by humans and coding agents together. Decisions that are hard to reverse (protocol shapes, storage formats, security boundaries) get rediscovered or silently undone when the reasoning lives only in chat history.

## Decision

We record architecturally significant decisions as short Markdown files in `brok/docs/adr/`, numbered `NNNN-kebab-title.md`, for every repository in the organization. Each record has **Status**, **Context**, **Decision** and **Consequences**. A decision is changed by adding a new record that supersedes the old one, never by editing history.

Write an ADR when a change is a one-way door: wire protocol, `shard.json` fields, on-disk formats, public CLI behaviour, security boundaries, or a dependency that constrains the build.

## Consequences

- Agents and newcomers can read why things are the way they are before proposing to change them; skills such as `grill-with-docs` and `domain-modeling` read and update ADRs inline.
- The `.out-of-scope/` directory complements ADRs: ADRs record what we chose, `.out-of-scope/` records what we declined.
