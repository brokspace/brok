# 0008. Supabase as the Shard Hub backend

- **Status**: Accepted
- **Date**: 2026-11-22

## Context

The Hub needs identity, a relational catalog, row-level authorization, a few lightweight endpoints, durable job queues and scheduled work. The team is small; operating a custom API server, Redis and a worker fleet before there is a single user is the wrong first investment.

## Decision

Use Supabase: PostgreSQL as the source of truth, Supabase Auth with GitHub OAuth for sign-in, Edge Functions for lightweight operations, PGMQ for durable asynchronous jobs, and ephemeral GitHub Actions runners for anything that builds or analyses third-party code. The service role key is server-side only. All schema changes are SQL migrations in Git, and row-level security is on for every exposed table.

## Alternatives considered

- **Self-hosted Postgres plus an API service**: full control, but adds operations and auth work before product validation.
- **Firebase or a document store**: weaker fit for the relational model (namespaces, versions, evidence) and for RLS-style authorization.
- **Dedicated queue (Redis/BullMQ)**: unnecessary while PGMQ meets the volume.

## Consequences

- Fast MVP, but Supabase becomes a dependency. Keep business rules in plain SQL and typed Rust or TypeScript modules behind the provider contracts, so a move to self-hosted Postgres stays a deployment change, and document that exit path in the Hub repository.
- The public Hub repository includes migrations, RLS policies and Edge Functions, so security-critical code is reviewable ([0013](0013-open-core-and-repository-boundaries.md)). Secrets and production configuration never enter Git.
- Tenant isolation is a tested property: every table with a policy has an isolation test (brokspace/brok#80).
- Edge Functions never run package code; builds and analysis happen only in isolated CI.
