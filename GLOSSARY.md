# Brokspace

A local-first platform that indexes a development workspace, runs small event-driven **Shards** on a local **Bus**, and connects the workspace to coding agents and LLMs through **Integrations** and **Skills**. This glossary is the shared language for code, docs, issues and agent sessions across every Brokspace repository.

## Language

### Workspace and index

**Workspace**:
A directory tree (usually a git repository, possibly with nested submodules) that `brok init` has prepared. Its root is the directory holding the **Brokspace dir**.
_Avoid_: project, repo (when you mean the whole tree including submodules)

**Brokspace dir**:
The `.brokspace/` directory at the **Workspace** root: `brok.yml`, model cache, databases, installed **Shards**, runtime state and logs.
_Avoid_: config folder, data dir

**Partition**:
An independently stored slice of the **Index**, cut along git submodule boundaries. The root repository is the **Cold partition**; each submodule is a **Hot partition** with its own small database.
_Avoid_: shard (a shard is a process, never a slice of the index), segment

**Cold partition**:
The **Partition** of the root repository, stored under `databases/cold/`. Searched with reduced weight when the user works inside a submodule.

**Hot partition**:
The **Partition** of one git submodule, stored under `databases/hot/<id>/`. Edits inside a submodule only rewrite its hot partition.

**Chunk**:
The retrieval unit of the **Index**: one function, method, class, impl block, Markdown section or text block, with its path, line range, symbol, parent and signature.
_Avoid_: snippet, fragment, document

**Index**:
All **Partitions** of a **Workspace**: chunk store plus BM25 index plus the file manifest that makes indexing incremental.
_Avoid_: database (when you mean the whole index), vector store

**Hybrid search**:
Search that fuses BM25 text ranking and vector ranking with Reciprocal Rank Fusion. Degrades to BM25 only when embeddings are off.
_Avoid_: semantic search (that is only the vector half)

### Bus and events

**Bus**:
The local event broker started by `brok run`, reachable over a Unix socket or Windows named pipe. Authenticates every connection and enforces **Grants**.
_Avoid_: message queue, broker (in prose), daemon (the daemon is `brok run`, which hosts the bus)

**Event**:
One message on the **Bus**: an id, a **Topic**, an optional correlation id, the authenticated source and a JSON **Payload**.
_Avoid_: message, notification

**Topic**:
The routing key of an **Event**, written `namespace:name` (`file:changed`, `agent:ask.chunk`). The `system:` namespace is reserved for the runtime.

**Topic pattern**:
An exact **Topic** or a prefix ending in `*` (`file:*`, `agent:ask.*`, `*`), used in subscriptions and **Manifests**.
_Avoid_: wildcard topic, filter

**Payload**:
The typed body of an **Event**. Well-known payloads live in `brok_sdk::payload`; shards define their own with the `Payload` trait.

**Request and reply**:
An **Event** published as a request; any **Event** carrying its id as correlation id is a reply and is delivered only to the requester.

### Shards and skills

**Shard**:
A standalone program, written by anyone, that joins the **Bus**, reacts to **Events** and publishes new ones. Described by a **Manifest** and installed with `brok shard install`.
_Avoid_: plugin, extension, agent (unless it is the **System shard**), worker

**System shard**:
The default **Shard** that ships with Brokspace: `brok-agent` (change tracking, pre-indexation review, RAG answers).
_Avoid_: core agent, built-in agent

**Skill shard**:
A **Shard** that contributes **Skills** (its **Manifest** lists `skills` directories). It may have no entrypoint at all, in which case it never runs as a process.

**Manifest**:
The `shard.json` file declaring a **Shard**'s identity, entrypoint, **Capabilities**, topic permissions, skills, build command and release binaries.
_Avoid_: config, package.json, descriptor

**Capability**:
A permission a **Shard** requests in its **Manifest**: topics to publish and subscribe, files to read and write, network, programs, environment variables. Shown to the user for consent at install time.
_Avoid_: scope, privilege

**Grant**:
What the **Bus** actually allows a session to do, derived from a **Manifest** (for shards) or built in (for the **Operator** and runtime services).

**Operator**:
The local user acting through the `brok` CLI. Its **Grant** covers every non-system **Topic**.
_Avoid_: admin, root, user (when you mean the bus identity)

**Skill**:
A reusable instruction set for an agent in the Agent Skills format: a folder with a `SKILL.md` (frontmatter `name` and `description`) and optional reference files. Built-in skills ship inside `brok`; more come from **Skill shards** or the workspace's own `.brokspace/skills/`. Skills run in coding agents (after `brok skill export`) or through the **System shard** with `brok skill run`.
_Avoid_: prompt, recipe, command, template

**User-invoked skill**:
A **Skill** reachable only when a human types its name (`/to-spec`). Orchestrates a flow. Never invoked by another skill.

**Model-invoked skill**:
A **Skill** an agent may reach for on its own when the task fits (`tdd`, `brok-context`). Holds a reusable discipline.

### Integrations

**Integration**:
A connection between Brokspace and a tool outside it: coding agents (through **Skills** and the MCP server), local or hosted LLM **Providers**, git, object storage.
_Avoid_: connector, adapter (in prose)

**Provider**:
An OpenAI-compatible chat completions endpoint configured under `llm.providers` in `brok.yml`: Ollama, LM Studio, Anthropic, DeepSeek, and others.
_Avoid_: model (a provider serves models), backend, vendor

**MCP server**:
`brok mcp`, the stdio Model Context Protocol server that exposes **Hybrid search** and workspace status to coding agents as tools.

## Relationships

- A **Workspace** has one **Brokspace dir** and one **Index**.
- An **Index** has exactly one **Cold partition** and zero or more **Hot partitions**.
- A **Partition** holds many **Chunks**; a **Chunk** belongs to exactly one **Partition**.
- The **Bus** carries **Events**; every **Event** has one **Topic** and one **Payload**.
- A **Shard** has one **Manifest**; its **Grant** is derived from the **Manifest**'s **Capabilities**.
- A **Skill shard** provides one or more **Skills**; a **Skill** is either **User-invoked** or **Model-invoked**.
- The **System shard** answers questions using one **Provider** at a time.

## Flagged ambiguities

- "shard" was once used for slices of the index. Resolved: a slice of the index is a **Partition**; **Shard** always means a process on the **Bus**.
- "agent" means three different things in the wild: the **System shard** (`brok-agent`), a coding agent (Claude Code, Codex, Cursor, Copilot) and an LLM. Resolved: say **System shard**, "coding agent" or **Provider** / model explicitly.
- "plugin" is avoided: Brokspace extensions are **Shards** (processes) or **Skills** (instructions), and the distinction matters for security and installation.
- "semantic search" is only the vector half of **Hybrid search**; use "hybrid search" for what `brok search` does.
