# Scope

Brokspace exists to make a development workspace more productive with as little ceremony as possible: install, run `brok init`, and the index, the local LLM, the agent and your coding agents just work. Ideas are welcome, and each one is judged against the bar below. Issues track changes to the project. Questions and open-ended discussion go to [GitHub Discussions](https://github.com/brokspace/brok/discussions).

This file applies to every repository in the [brokspace](https://github.com/brokspace) organization.

## The philosophy

1. **Simple and zero-config.** The default path works without editing a file. Every new option must earn its place; a good default beats a knob.
2. **Local-first.** The index, the embedding model and the bus run on the user's machine. Nothing leaves it unless the user configured a provider or a sync target.
3. **Shards and integrations over core features.** The core is a small, sharp runtime: index, bus, shard lifecycle, skills, integrations. Domain-specific behaviour (a security scanner, a Jira bot, a team's review policy, a new language server) is a **shard** or a **skill**, not a core feature.
4. **Open standards.** OpenAI-compatible LLM APIs, Agent Skills (`SKILL.md`), the Model Context Protocol, git. We integrate with what developers already use.
5. **Safe by default.** Shards get only what their manifest declares and the user approved.

## The bar

An issue stays open only if it clears **both** parts:

1. **Observed problem.** It describes something that went wrong or got in the way in real use: what you ran, what happened, what you expected. A hypothetical improvement ("it would be nice if...") does not clear this part.
2. **Fits the philosophy.** It does not match anything in [`.out-of-scope/`](./.out-of-scope/) or a past rejection, and it is not one of these:
   - a config option that serves one person's workflow (put it in your own shard or skill);
   - behaviour that could live in a shard or skill without changes to the core;
   - a harness-specific branch for a single coding agent when an open standard covers it.

Size is irrelevant: a one-word fix and a rewrite face the same bar. Reactions and +1 comments do not count towards it.

## Core or shard?

| It belongs in the core when... | It belongs in a shard or skill when... |
|---|---|
| every workspace benefits from it | it serves a domain, a team or a tool |
| shards cannot do it with the SDK | it can be built on top of the bus and the index |
| it is about the runtime contract (bus, manifest, index, lifecycle) | it is about what to do with events or code |

When a shard cannot be built because the SDK lacks something, that gap is in scope: open an issue on [brok-sdk](https://github.com/brokspace/brok-sdk/issues) describing the shard you are building.

## By repository

- **brok**: the bar above.
- **brok-sdk**: the bar above, plus compatibility: protocol and manifest changes are additive unless an ADR covers the migration.
- **brok-agent**: the bar above; the default agent stays general.
- **brok-shard-template**: kept minimal on purpose; bigger examples are their own repositories.

## Already decided

Each file in [`.out-of-scope/`](./.out-of-scope/) records one rejected concept and why. Read them before filing:

- [`built-in-feature-instead-of-shard.md`](./.out-of-scope/built-in-feature-instead-of-shard.md)
- [`cloud-hosted-service.md`](./.out-of-scope/cloud-hosted-service.md)
- [`config-options-for-one-workflow.md`](./.out-of-scope/config-options-for-one-workflow.md)
- [`non-openai-compatible-llm-apis.md`](./.out-of-scope/non-openai-compatible-llm-apis.md)

Architecture decisions that shaped the project live in [`docs/adr/`](./docs/adr/).

## Unclear issues

An issue that can't be judged against the bar gets one round of questions and the `needs-info` label. With no reply from the reporter in 14 days it is closed automatically; a reply reopens triage. Rejections close as "not planned".
