# Architecture

This document explains how the pieces of Brokspace fit together. It is aimed
at contributors and shard authors.

## Crates and repositories

```text
brok-sdk  (own repo, crates.io)   protocol · manifest · transport · client
   ▲
   ├── brok-core      workspace discovery, .brokspace/ layout, brok.yml, partitions, git
   ├── brok-bus       event bus: listener, sessions, router actor, token auth / ACL
   ├── brok-index     chunking, Arrow schema, BM25, stores, embeddings, search, index service
   ├── brok-shard     registry, installer, sandbox environment, supervisor
   └── brok-cli       the `brok` binary: one module per subcommand
brok-agent (own repo)            default system shard, depends only on brok-sdk
```

Rules of thumb: library crates use `thiserror`/`anyhow::Result` with context
and never print; only `brok-cli` talks to the terminal. Optional native
backends are cargo features, so the pure-Rust build always works.

## Workspace layout

```text
<workspace>/.brokspace/
├── brok.yml                configuration
├── .gitignore              runtime state is never committed by accident
├── cache/models/           ONNX weights + tokenizer
├── databases/
│   ├── .lock               exclusive writer lock (brok index / brok run)
│   ├── cold/               root repository partition
│   └── hot/<submodule-id>/ one micro-database per git submodule
│       ├── files.json      manifest: path → sha256, size, mtime, chunk count
│       ├── bm25.bin        BM25 index (postcard)
│       └── chunks.lance/ | CURRENT + chunks-NNNNNNNN.arrow
├── shards/
│   ├── shards.lock         provenance + binary sha256 of every shard
│   └── <name>/{shard.json, bin/…}
├── run/                    operator.token (0600), brok.pid, brok.sock (Unix)
└── logs/<shard>.log
```

## Source-control providers

Brok talks to source control through the `SourceControl` trait in `brok-core` (`provider.rs`, [ADR 0015](adr/0015-provider-registry-and-source-control-contract.md)). Git is the built-in, default provider and wraps the same `git` calls as before, so single-repository and Git workflows behave exactly as they did. Using brok never requires configuring a provider.

A provider declares a contract version and its capabilities (`status`, `history`, `submodules`, and later `diff`, `watch`, `worktrees`). Only `is_repository` and `nested_repositories` are required; any other operation on a provider that lacks the capability returns `ProviderError::Unsupported`, never an empty success. Revisions are opaque strings. Provider settings may hold credential references, never secret values.

To add a provider: implement `SourceControl`, give it a unique lowercase id and `CONTRACT_VERSION`, register it with `ProviderRegistry::register` (duplicate ids and other contract versions are rejected with a message naming the cause), and test it against the same behavior the Git tests pin: repository detection, nested repositories, a repository without commits, and the `Unsupported` paths. Changing `CONTRACT_VERSION` needs a new ADR. Link management (`brok link`, which clones and fetches) and `brok setup lfs` remain Git-specific on purpose.

## Indexing pipeline

1. **Walk** with the `ignore` crate (nested `.gitignore`s, global excludes),
   then apply `index.include` / `index.exclude`.
2. **Route** each file to the deepest partition containing it
   (`PartitionMap::resolve`).
3. **Skip unchanged files** cheaply: same size and mtime as in `files.json` →
   untouched; otherwise hash the content and compare.
4. **Chunk** in parallel (rayon). The AST chunker emits one chunk per
   definition, attaches doc comments/attributes/decorators, splits large
   containers into a header plus members (`parent` = the container) and
   groups loose top-level code into `preamble` chunks.
5. **Embed** in batches on a blocking thread (`spawn_blocking`), prefixing
   `search_document:` as nomic models expect; mean pooling → layer norm →
   Matryoshka truncation → L2 normalization.
6. **Write**: delete the file's previous chunks, append the new ones, update
   BM25 and the manifest, then commit atomically. Files are processed in
   batches of 512 so memory stays bounded on large repositories.

`brok index` and `brok run` take an exclusive OS file lock on
`databases/.lock`; readers (`brok search`, `brok status`) never block.

### Stores

Both stores share one Arrow schema (`brok-index/src/schema.rs`): `id`, `path`,
`language`, `kind`, `symbol`, `parent`, `signature`, line and byte ranges,
`file_hash`, `content` and an optional `FixedSizeList<Float16>` `vector`
(`Float32` in indexes created by brok 0.10 and earlier, until rebuilt:
[ADR 0017](adr/0017-compact-vector-storage.md)).

- **LanceDB**: deletes/appends go straight to the table; compaction runs every
  16 writes and an IVF-PQ index with the cosine metric is built past 10 000
  rows. Queries re-rank ten times the wanted results with the stored vectors
  ([measurements](benchmarks.md#vector-index)).
- **Two passes** ([ADR 0018](adr/0018-text-first-indexing.md)): changed files
  are first stored and added to the keyword index without vectors, so they are
  searchable at once. Vectors are then computed and saved about once a minute
  (whole files; a long run that is interrupted resumes from the last save). A
  partition is searched by meaning once 95% of its chunks have a vector. The
  daemon holds the index only while saving, and takes over from a `brok index`
  that was writing when it started.
- **Arrow IPC**: each commit writes a new immutable generation file and flips
  `CURRENT`. Files are memory-mapped and decoded zero-copy; immutability is
  what makes `mmap` sound and lets readers keep an old generation (Windows
  cannot replace a mapped file). Vector search is exact (dot product over
  normalized vectors).

## Search

For each partition: BM25 top-k and vector top-k (when the partition has
vectors of the right size) → Reciprocal Rank Fusion (`k = 60`), normalized so
that rank 1 in every list scores 1.0. Partitions other than the focus (the
partition of the current directory) are multiplied by `search.cold_weight`.
The final top-k chunk ids are materialized from their stores.

## Event bus

- **Transport**: Unix domain socket at `.brokspace/run/brok.sock` (falls back
  to `$XDG_RUNTIME_DIR` for long paths) or the named pipe
  `\\.\pipe\brok-<hash of the workspace path>`. Frames are a 4-byte big-endian
  length plus JSON, capped at 16 MiB.
- **Handshake**: `Hello{protocol, name, version, token}` → `Welcome{session,
  identity, context}` or `Error`. Tokens are 256-bit random, stored hashed.
- **Router**: a single actor task owns all state, so routing is lock-free.
  Delivery uses bounded queues and `try_send`: a slow client loses events (and
  is logged) rather than stalling everyone.
- **Semantics**: topic patterns (`file:*`), `source` stamped by the bus,
  `system:*` reserved and broadcast, request/reply via `correlation_id` routed
  only to the requester (a bounded table with a 15 minute TTL).

### Built-in participants

| Participant | Identity | Publishes | Subscribes |
|---|---|---|---|
| CLI (`brok ask`, `brok bus`, `brok search`) | `operator` | any non-system topic | `*` |
| Index service | `brok-index` | `index:*` | `file:changed`, `index:search` |
| Shards | their manifest name | `publishes_to` | `subscribes_to` |

### Well-known topics

| Topic | Payload | Direction |
|---|---|---|
| `file:changed` | `{changed: [path], deleted: [path]}` | agent → index (and anyone) |
| `index:search` / `index:search.result` | `SearchRequest` / `SearchResults` | request/reply |
| `index:updated` | counts and duration | index → anyone |
| `agent:ask` | `{prompt, provider?, context_chunks?}` | CLI → agent |
| `agent:ask.chunk` / `.done` / `.error` | streamed reply | agent → requester |
| `review:result` | `{path, verdict, summary}` | agent → anyone |
| `system:shutdown` | `{reason}` | runtime → everyone |
| `system:request.cancelled` | `{request}` | runtime → the agent, when the client asking an `agent:ask` leaves before the answer is complete |

Types live in `brok_sdk::payload`.

## Daemon lifecycle

`brok run` is the daemon in the foreground; `brok start` launches the same
process detached (output in `.brokspace/logs/brok.log`), and `brok ask`,
`chat`, `review`, `commit` and `skill run` start it on demand. Readiness means
the bus is listening and, when installed, `brok-agent` is connected; the CLI
checks it with a `daemon:status` request.

Start: bind the bus → issue the operator token (`run/operator.token`) → start
the control service (`daemon:status`, `daemon:stop`) → open the index for
writing (falls back to read-only search if the model or lock is unavailable),
catch up on changes made while stopped, and connect the index service → for
each enabled shard, issue a token from its manifest, build its sandbox
environment and supervise it.

Stop (Ctrl+C, or `brok stop` publishing `daemon:stop`): broadcast
`system:shutdown` → shards get 3 seconds to exit, then are killed → index
service stops → bus stops → runtime files removed.

Freshness: `brok-agent` reports dirty files every few seconds and, when a
repository's `HEAD` moves (commit, pull, checkout), the files that differ
between the two commits; root, submodules and nested repositories alike.

Crashing shards restart with exponential backoff (1 s … 30 s); five crashes
within a minute and the supervisor gives up.

## Memory profile

- Text-only search (embeddings disabled or model missing): BM25 postings in
  memory plus memory-mapped chunk data; typically a few tens of MB.
- Semantic search loads the quantized model (~140 MB of weights) once per
  process. Run `brok run` and searches reuse its already-loaded model through
  the bus instead of loading their own.
- Indexing streams files in batches; the Arrow store keeps staged batches in
  memory until commit, LanceDB writes per batch.
