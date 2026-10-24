# 0006. MCP stdio server as the coding-agent integration surface

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

Coding agents are where developers spend their day. They should be able to use the workspace index (hybrid search over AST chunks) without each agent needing a bespoke plugin.

## Decision

`brok mcp` runs a Model Context Protocol server over stdio that exposes Brokspace as tools (`brok_search`, `brok_status`). It reuses a running `brok run` daemon through the bus when there is one (model already loaded) and otherwise opens the index read-only itself. `brok setup agents` writes the project-level MCP configuration for the supported agents, together with the exported skills.

## Consequences

- One integration covers every MCP-capable agent; skills such as `brok-context` teach agents when to call the tools.
- The MCP tool surface is a public interface: renaming tools or changing their arguments is a one-way door that needs a new ADR.
- Long-running work stays in `brok run`; the MCP server is a thin front end.
