# Benchmarks and resource budgets

`brok-bench` (`crates/brok-bench`) measures what brok costs a developer's
machine, so every optimization starts from a number and ends with a
before/after. It is a developer tool: it is not part of the `brok` binary and
is not published.

```sh
cargo build --release -p brok-cli --no-default-features   # or with default features
cargo run --release -p brok-bench -- run --brok target/release/brok --size small --out result.json
cargo run --release -p brok-bench -- compare crates/brok-bench/baselines/small-text.json result.json
```

`brok-bench list` prints the scenarios, `brok-bench fixture <dir>` generates a
workspace to experiment on by hand. Run `--help` on each command for flags.

## What it runs

Every scenario drives the real `brok` binary against a **synthetic
workspace** in a temporary directory, with its own user configuration (your
`~/.config/brokspace` is never touched) and, on Unix, a counting `git` first on
`PATH`.

| Scenario | What it measures |
|---|---|
| `init` | `brok init` in a fresh workspace (no model download, no agent) |
| `index-initial` | the first `brok index` |
| `index-noop` | `brok index` when nothing changed (repeated filesystem scans) |
| `index-incremental` | `brok index` after one file changed |
| `search` | 20 `brok search` runs; `p50_ms` and `p95_ms` include process startup |
| `idle-daemon` | `brok run` left alone for `--idle-secs` after a 3 s warm-up |
| `multi-repo` | the first index of a workspace with 3 nested repositories and 1 submodule |
| `multi-workspace-idle` | 3 workspaces running their daemons at once |
| `llm-answer` | **skipped**: needs a configured LLM; token accounting is brokspace/brok-agent#23 |

The default mode is **text only** (`embeddings.enabled: false`): it is
offline and reproducible. `--semantic` also downloads the embedding model once
(reuse it with `--models-dir`). `--with-agent` installs the default
`brok-agent` (network) so the idle scenario includes its Git polling.

### Fixtures

Generated from a fixed seed, so the same size is byte-identical everywhere:
Rust, Python, TypeScript, Go, Java and Markdown files with identifiers from a
small vocabulary (realistic term overlap for BM25).

| Size | Files | Source |
|---|---:|---:|
| `small` | 50 | 0.1 MiB |
| `medium` | 1,500 | 2 MiB |
| `large` | 15,000 | 20 MiB |

### Metrics

Per scenario: wall time, CPU time (user + system), peak resident memory, disk
bytes read and written, `git` processes started, size of `.brokspace/databases`
and of all of `.brokspace`. Idle scenarios also record the steady-state memory,
as opposed to the startup peak.

How the numbers are obtained, and what they mean:

- **One-shot commands** run under `/usr/bin/time` (GNU on Linux, BSD on macOS):
  the OS reports exact peak memory, CPU time and block I/O, even for a command
  that lives 5 ms. Peak memory is that of the largest process of the tree.
- **Daemons**, and one-shot commands where `time` is missing (Windows,
  minimal containers), are **sampled** every 25 to 100 ms with `sysinfo`. A
  process that lives less than one interval can be missed; threads are not
  counted as processes (Linux reports each thread with the memory of the whole
  process).
- Disk bytes are 0 where the OS does not expose them per process.
- **Git processes** are counted by a `git` shim on `PATH` (Unix only; the
  field is absent on Windows). It sees every `git` that brok and its shards
  start, and none started before the measuring window.
- `unsafe` is denied in this workspace, so the tool cannot call `getrusage`
  itself; that is why it relies on `time` and on sampling.

### Privacy

A report holds scenario names, counts, sizes and timings, nothing else. The
fixtures are synthetic, so no real source code, prompt or answer is involved,
and a test pins the exact set of fields a report can contain, so adding one
is a reviewed change.

## Reading the report

`result.json` has `schema_version: 1`. A scenario's `status` is `passed`,
`failed` or `skipped` (with a `reason`); a scenario that did not run is never
reported as passed. Fields that could not be measured are absent, not zero.

## Regression checks

`brok-bench compare baseline.json result.json` exits 1 when a **gated** metric
gets worse than allowed:

| Metric | Gate |
|---|---|
| `index_bytes` | grows by more than 10 % (and 4 KiB) |
| `git_subprocesses` | grows by more than 10 % and by at least 3 |
| scenario status | a scenario that passed in the baseline no longer passes |
| `peak_rss_bytes` | informational; gated only with `--memory-tolerance 0.5` |
| `cpu_ms`, `wall_ms` | informational (shared CI runners are noisy) |

Index size and the number of `git` processes are deterministic, so they are
safe to fail a build on. The two reports must have the same fixture size and
mode, otherwise the comparison is refused.

The **Bench** workflow builds the lean release binary, runs the `small`
fixture and compares it with `crates/brok-bench/baselines/small-text.json`. It
runs on pull requests that touch `crates/`, on `staging`, weekly and on demand,
and uploads the report as an artifact. It is not a required check.

To update a baseline after an intended change, run the benchmark on the
reference machine and commit the new JSON in the same PR, with the reason in the
description.

## Baseline (text mode, brok 0.4.1)

Measured on Linux x86_64, Intel i9-10900KF (20 logical CPUs), 32 GB RAM, lean
release build. Absolute times are for this machine; the point is the relative
size of each cost.

| Scenario | small | medium | large |
|---|---:|---:|---:|
| `index-initial` wall / CPU | 14 ms / 10 ms | 91 ms / 130 ms | 920 ms / 1,320 ms |
| `index-initial` peak memory | 13.2 MiB | 22.6 MiB | 127.1 MiB |
| index size on disk | 0.1 MiB | 3.1 MiB | 31.4 MiB |
| `index-noop` wall / CPU | 4 ms / 0 ms | 13 ms / 10 ms | 121 ms / 120 ms |
| `index-incremental` wall | 11 ms | 32 ms | 244 ms |
| `search` p50 / p95 (process) | 3 / 3 ms | 9 / 10 ms | 82 / 87 ms |
| idle daemon memory (steady) | 10.9 MiB | 17.5 MiB | 74.3 MiB |
| idle daemon CPU over 15 s | 0 ms | 0 ms | 0 ms |
| idle `git` processes over 15 s | 0 | 0 | 0 |
| 3 idle workspaces together | 32.6 MiB | 32.4 MiB | 35.5 MiB |
| `multi-repo` (5 repositories) index size | 0.2 MiB | 4.4 MiB | 44.3 MiB |

What the numbers say:

- The core daemon is **quiet when idle**: no CPU and no `git` process, in
  every size. Its memory grows with the index, which is memory-mapped.
- A no-op `brok index` on the large workspace still costs 120 ms of CPU: the
  metadata check scans every file. That is the "repeated filesystem scans" cost
  of brokspace/brok#78, and the target for event-driven tracking.
- Indexing the large workspace peaks at 127 MiB, about 4 times the index size
  on disk.
- **With brok-agent installed** (`--with-agent`, medium fixture, 30 s idle): the
  daemon started **18 `git` processes** while nothing changed, from the agent's
  5-second polling (brokspace/brok-agent#22). Memory was 24.5 MiB in total.

Not measured yet, on purpose: semantic search (needs the model; run with
`--semantic` and add its baselines), LLM answers (brokspace/brok-agent#23), and
Windows and macOS (the tool runs there, but the committed baselines are Linux).
