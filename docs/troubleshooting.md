# Troubleshooting, metrics and bug reports

When something is slow, wrong or broken, four commands tell you why, and a
fifth reports it with everything a maintainer needs.

| Command | Answers |
|---|---|
| `brok doctor` | Is everything installed and reachable? Each problem comes with the command that fixes it. |
| `brok status` | What is indexed, linked and running right now. |
| `brok logs -f` | What the background brok is doing (indexing, the agent, shards). |
| `brok stats` | How searches, answers and indexing performed lately, and what failed. |
| `brok bug` | A report with all of the above, secrets masked, and a prefilled issue. |

Add `-v` (or `-vv`) to any command for more detail on stderr, and `--json` to
`doctor`, `status` and `stats` for scripts and editors.

## Common problems

| Symptom | Likely cause | Fix |
|---|---|---|
| `brok ask` says "no answer from the agent" | The LLM is not running or not reachable | Start Ollama or LM Studio; `brok doctor` shows the URL brok tries |
| Answers are slow to start | A large local model on the CPU | `brok stats` shows the time to the first words; pick a smaller model with `brok setup llm` |
| Search misses recent edits | The background brok is stopped | `brok start` (it follows your edits); or `brok index` |
| Search only matches words | Embeddings are off or the model is missing | `brok model download`, then `brok index` |
| Results from a linked workspace are missing | The link is not indexed | `brok link list`, then `brok link sync <name>` |
| Memories are not found | Memory is off | `brok config get memory`; `brok config set memory.enabled true` |
| "another brok process is writing the index" | `brok run` is indexing | Wait, or `brok stop` before `brok index --rebuild` |

## Metrics

brok records how long searches, answers and indexing take, and which
commands failed, in `.brokspace/metrics/` (one JSON line per event, one file
per month). They stay on your machine: nothing is sent anywhere.

```sh
brok stats            # the last 7 days
brok stats --days 30
brok stats --json     # for scripts and editors
```

```text
Last 7 days
  search    412 searches · p50 38 ms · p95 210 ms · 6 with no results · 40 used linked workspaces · 51 found memories
  answers   37 answers · p50 14.2 s · p95 41.0 s · 2 failed · first words 3.1 s · ollama/qwen2.5-coder:7b (35)
  index     88 runs · p50 180 ms · p95 2.4 s · 1204 files indexed · last 2026-11-03 10:12
  errors    3 · ask (2), link (1)
    2026-11-03 09:58 ask: no answer from the agent. Check `brok doctor`...
```

What is recorded, by event:

| Event | Fields |
|---|---|
| `search` | duration, hits, whether embeddings were used, hits from linked workspaces, memories |
| `answer` | duration, time to the first words, provider and model, skill, success, number of earlier turns |
| `index` | duration, files scanned, indexed and removed, chunks, full run or live update |
| `error` | the command and the first line of its message (secrets masked) |

Queries, answers and file contents are never recorded. Turn metrics off with
`brok config set metrics.enabled false`; months older than
`metrics.retention_days` (90) are deleted when brok starts.

## Reporting a bug

```sh
brok bug
brok bug --title "review hangs on large diffs" --repo brok-agent
```

`brok bug` asks for a one-line summary, then builds a report with:

- brok's version and backends, the OS, the LLM (local or hosted, never its
  key), the embedding model, links, memory and daemon state;
- the output of `brok doctor`;
- the last 7 days of metrics, with the latest errors;
- the last 60 lines of the log and your `brok.yml`.

Secrets are masked and your home directory shows as `~`. The report is
printed and saved to `.brokspace/reports/`; read it before you share it.
brok then opens the repository's bug form with the summary and diagnostics
filled in. Nothing is sent until you submit the issue on GitHub; attach the
report file for the log and the configuration.

| `--repo` | For problems with |
|---|---|
| `brok` (default) | The CLI, indexing, search, memory, links, MCP |
| `brok-agent` | Answers, reviews, commit messages, skills on your LLM |
| `ide` | Brokspace IDE (discontinued: editor extensions replace it) |

Editors call `brok bug --json` (with `--title`, `--what-happened`,
`--context` for their own versions) and open the returned `url` themselves.
Security issues never go in a public issue: see the repository's security
policy.
