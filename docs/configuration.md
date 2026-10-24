# Configuration reference

`brok init` writes `.brokspace/brok.yml` with comments. Every key is optional;
missing keys take the defaults below. Unknown keys are rejected with a
"did you mean" style error, so typos never go unnoticed.

Changes take effect on the next command (or after restarting `brok run`).
Changing `embeddings.model`, `embeddings.dimensions` or `index.store`
triggers a rebuild of the affected partitions on the next `brok index`.

## `workspace`

| Key | Default | Description |
|---|---|---|
| `submodules` | `true` | Index git submodules and git repositories nested in the workspace (up to 4 levels, e.g. a folder of repos) as independent *hot* partitions. |

## `index`

| Key | Default | Description |
|---|---|---|
| `include` | `["**/*"]` | Globs (relative to the workspace root) of files to index. |
| `exclude` | `.git`, `.brokspace`, `target`, `node_modules`, `dist`, `build`, `*.lock`, `*.min.js` | Globs to skip. `.gitignore` files are always honored on top of this. |
| `max_file_kb` | `512` | Larger files are skipped. Binary files (NUL bytes) and non UTF-8 files are always skipped. |
| `max_chunk_lines` | `120` | A syntactic unit longer than this is split: containers into members, other nodes at line boundaries. |
| `store` | `auto` | `auto` (LanceDB when compiled in), `lancedb` or `arrow` (memory-mapped Arrow IPC, exact vector search). |

## `embeddings`

| Key | Default | Description |
|---|---|---|
| `enabled` | `true` | `false` runs on BM25 text search only: no download, minimal memory. |
| `model` | `nomic-embed-text-v1.5` | The only supported model today. |
| `variant` | `quantized` | `quantized` (~140 MB, int8) or `full` (~550 MB, fp32). |
| `dimensions` | `768` | Matryoshka truncation: `64`, `128`, `256`, `512` or `768`. |
| `device` | `auto` | `auto`, `cpu`, `cuda`, `coreml`, `directml` (GPU devices need the matching build feature). |
| `batch_size` | `16` | Texts per inference batch. |

Model files are downloaded once per user and shared by every workspace:
`~/.cache/brokspace/models` (Linux), `~/Library/Caches/brokspace/models`
(macOS), `%LOCALAPPDATA%\brokspace\models` (Windows). Set `BROK_MODELS_DIR`
to use another directory. A copy inside `.brokspace/cache/models/` (from
older versions) keeps working.

## User defaults

`brok setup llm` also stores your LLM choice in the user configuration
(`~/.config/brokspace/config.yml`, `~/Library/Application Support/brokspace/`
or `%APPDATA%\brokspace\`; override with `BROK_CONFIG_DIR`). `brok init`
applies it to every new workspace, so you choose a model once per machine.

## `search`

| Key | Default | Description |
|---|---|---|
| `limit` | `8` | Default number of results. |
| `cold_weight` | `0.85` | Score multiplier for partitions other than the one you run the command from. |

## `llm`

OpenAI-compatible chat completion endpoints used by `brok-agent`.

```yaml
llm:
  default: ollama
  providers:
    ollama:   { base_url: http://localhost:11434/v1, model: qwen2.5-coder:7b }
    lmstudio: { base_url: http://localhost:1234/v1,  model: qwen2.5-coder-7b-instruct }
    anthropic:
      base_url: https://api.anthropic.com/v1
      model: claude-sonnet-5-5
      api_key_env: ANTHROPIC_API_KEY
      max_tokens: 2048
```

| Key | Description |
|---|---|
| `default` | Provider used unless `brok ask --provider` says otherwise. Must exist in `providers`. |
| `providers.<name>.base_url` | URL up to and including `/v1`. |
| `providers.<name>.model` | Model name as the provider expects it. |
| `providers.<name>.api_key_env` | **Name** of the environment variable holding the key. Keys never go in this file. |
| `providers.<name>.temperature` / `max_tokens` | Optional sampling settings. |

## `sync`

Where the index is mirrored, configured with `brok setup sync`.

| Key | Description |
|---|---|
| `backend` | `none`, `s3`, `r2`, `minio` or `git-lfs`. |
| `remote.bucket` / `prefix` / `region` / `endpoint` | Object storage coordinates. |
| `remote.access_key_env` / `secret_key_env` | Names of the variables holding the credentials (default `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`). |

`git-lfs` tracks `.brokspace/databases/**` with Git LFS and stops ignoring it,
so the index travels with the repository. The object storage sync engine
(`brok sync`) is on the roadmap: today `brok setup sync` records the target.

## `shards`

```yaml
shards:
  brok-agent:
    enabled: true          # brok run starts it (shards not listed are enabled)
    settings:              # free-form, handed to the shard when it connects
      watch_interval_secs: 5
```

Each shard documents its own `settings`. For `brok-agent` see its
[README](https://github.com/brokspace/brok-agent#settings).

## Environment variables

| Variable | Used by | Purpose |
|---|---|---|
| `RUST_LOG` | all | Log filter, e.g. `RUST_LOG=brok_index=debug`. |
| `NO_COLOR` | CLI | Disable colors. |
| `BROK_MODELS_DIR` | CLI | Embedding model cache (default: the OS cache directory). |
| `BROK_CONFIG_DIR` | CLI | User defaults directory (default: the OS config directory). |
| `BROK_*` (others) | shards | Set by `brok run` for shard processes; reserved. |
