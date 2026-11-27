# 0018. Index text first, then add vectors in saved batches

- **Status**: Accepted
- **Date**: 2026-11-26

## Context

brokspace/brok#200. With semantic search, a partition is usable only when every chunk is embedded, and
it is saved only then. On a project of 147 files (1504 chunks) the first search waited about 150 s, and
Ctrl+C after 45 s lost everything and left `status` showing `0 files 512 chunks`. The same size of
project indexes as text only in a fraction of a second. The daemon also holds the index's write lock
for the whole catch up, so searches wait for it.

## Decision

1. **Two passes per partition.** The text pass chunks the changed files, writes their rows **without
   vectors**, the keyword index and the manifest, and marks those files as waiting for vectors. The
   vector pass then embeds them and replaces their rows.
2. **Checkpoints.** The vector pass saves whole files, about once a minute of embedding and at the end.
   A file stops waiting only after the store holding its vectors is committed. A cancelled or killed run
   resumes with the files still waiting. Old LanceDB versions are pruned when the pass ends.
3. **Search stays honest.** While fewer than 95% of a partition's chunks have vectors, it is searched
   by keywords only, and the result says so (`semantic: false`). Mixing a partly embedded partition
   into hybrid ranking would favour the files that happened to be embedded first. At 95% and above,
   as after a small edit, hybrid search stays on and the waiting chunks are found by keywords.
4. **No lock during embedding.** The daemon takes the write lock to apply a checkpoint, not while
   embedding, so searches are answered during indexing.
5. **No new background process.** `brok init` and `brok index` announce that text search works after the
   first pass and carry on in the foreground with progress; Ctrl+C keeps what is saved. A running
   daemon picks up waiting files.
6. **`status` reports the truth**: chunks, how many have vectors, and whether the partition is searched
   by meaning (brokspace/brok#201).
7. **Compatibility.** The vector column becomes nullable in new and rebuilt partitions, with partition
   manifest version 3, so brok 0.11 and earlier rebuild such a partition rather than misread it.
   Existing partitions keep their schema and today's single pass until `brok index --rebuild`.
8. Text only workspaces (`embeddings.enabled: false`) are unchanged.

No new setting.

## Evidence

A throwaway prototype on one Linux x86_64 machine: synthetic repetitive text and random 768 dimension
`float16` vectors, LanceDB 0.41, no real embedding.

- LanceDB accepts rows without vectors. A vector search then returns nothing, or only the rows that have
  vectors, without error, and the cosine index builds while some rows have none (20 000 and 100 000
  rows).
- Writing 100 000 text rows took 0.5 s (chunking and the keyword index not included).
- Replacing rows in batches, at 100 000 chunks:

| batch | time spent writing | size on disk afterwards | single pass, as today |
|---:|---:|---:|---|
| 512 | 68 s | 1262 MB | 1.4 s, 320 MB |
| 4096 | 13 s | 465 MB | |

- The Arrow store rewrites all its data at each commit: 0.9 s and 286 MB at 100 000 chunks.

Embedding on that machine runs at about 11 chunks a second, so 100 000 chunks take hours: a checkpoint a
minute adds around 1% of time in either store. It does add disk writes, and the extra LanceDB versions
take space until pruned.

## Measured on the implementation

Linux x86_64, 167 files and 1567 chunks, quantized model, five runs of each, LanceDB store.

| | single pass | two passes |
|---|---|---|
| Text search ready | when indexing ends, about 130 s | 0.4 to 0.9 s |
| Total time | 131 to 140 s | 132 to 152 s, paired runs within 9% (median 1%) |
| Peak memory | 564 to 568 MB | 581 to 602 MB, median 4% more |

The first version needed about 20% more memory at its peak: glibc kept memory freed by the text pass in
pools that embedding could not reuse. Indexing now returns freed memory to the system after the text
pass and after each save. What remains is file backed memory; the process' own allocations match the
single pass. Capping the number of pools (`M_ARENA_MAX`) fixed it too, but made parallel text indexing
20 to 36% slower, so it was not used. With the Arrow store the peak went down.

## To verify before acceptance of the implementation

- First search under 10 s on the 147 file reference project.
- Total index time and peak memory within 5% of today's single pass.
- Process killed during each pass and during a commit: no corruption, no lost saved work, no duplicate
  chunks, on both stores.
- The Arrow store skips rows without vectors, and its write volume on a large partition; lengthen its
  checkpoint interval as the store grows if needed.
- Disk after pruning, and recall of an index built while rows had no vectors.
- Search during indexing through the daemon; Windows file locking.
- Whether 95% is the right threshold, with real questions.

Still unverified: the 95% threshold has not been compared with other values on real questions. It is
kept for now, and is one constant to change.

## Consequences

- Text search in seconds, and indexing that can be interrupted.
- Search quality changes while indexing runs: keywords only until a partition is nearly complete.
- More code paths: two row states, a manifest version, and checkpoints.
- Upgrading an existing workspace to this behaviour costs a rebuild.

## Rejected

- **Hybrid ranking over a partly embedded partition**: biased towards whatever was embedded first.
- **A separate store for text until vectors arrive**: duplicates every chunk and adds a switch over.
- **Detaching a background process after the text pass**: surprises the developer and conflicts with
  avoiding unnecessary background processes.
