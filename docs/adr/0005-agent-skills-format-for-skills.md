# 0005. Agent Skills (SKILL.md) as the skill format

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

Brokspace wants reusable development workflows (grilling, specs, tickets, TDD, code review, triage) that work both with the user's coding agent and with a local LLM through the system shard. The coding-agent ecosystem (Claude Code, Codex, Cursor, GitHub Copilot, Gemini CLI) has converged on the Agent Skills format: a folder with a `SKILL.md` carrying `name` and `description` frontmatter, optional reference files, and harness metadata such as `agents/openai.yaml`.

## Decision

- Brokspace skills use the Agent Skills format unchanged, organised in buckets (`engineering/`, `productivity/`) and split into user-invoked (`disable-model-invocation: true`) and model-invoked skills, following the structure of [mattpocock/skills](https://github.com/mattpocock/skills).
- Built-in skills live in `brok/skills/` and are embedded in the `brok` binary, so they work offline with zero setup.
- More skills come from **skill shards** (a manifest field `skills` listing directories; no entrypoint needed) and from the workspace's own `.brokspace/skills/`.
- `brok skill export` installs skills into coding agents' skill directories; `brok skill run` executes a skill through the system shard with retrieved context and the configured LLM provider.

## Consequences

- Skills written for other agents work in Brokspace, and Brokspace skills work in other agents.
- Skill shards reuse the shard install, consent and provenance machinery (`shards.lock`).
- The built-in catalog is part of each release: skill changes follow the same review and changelog process as code.
