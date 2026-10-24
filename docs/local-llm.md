# Local LLM setup

Brokspace answers questions (`brok ask`) and runs skills (`brok skill run`)
with any OpenAI-compatible model. Running it locally keeps everything
**free, private and offline**. Search and indexing never need an LLM.

## The short version

```sh
brok setup llm      # also part of `brok init`
```

It:

1. reads your RAM (and NVIDIA GPU memory) and picks a model size from the
   table below;
2. looks for a running **Ollama** (`localhost:11434`) or **LM Studio**
   (`localhost:1234`) server;
3. uses a model you already have, or offers to download the recommended one
   (`ollama pull`);
4. writes the choice to `.brokspace/brok.yml` (`llm.default`).

Nothing installed yet? It prints the one-line install for your OS. Run it,
then `brok setup llm` again. `brok doctor` tells you at any time whether the
model is reachable.

## What your machine can run

Sizes are for 4-bit quantized models (`Q4_K_M`), the best balance between
speed and quality.

| Model size | RAM or VRAM | Typical hardware | Recommended (Ollama) |
|---|---|---|---|
| 1B to 3B | 8 GB RAM | Office laptops, entry-level Macs | `qwen2.5-coder:1.5b`, `llama3.2:3b` |
| **7B to 9B** (sweet spot) | 16 GB RAM, or a GPU with 6 GB+ | Mid-range PCs and laptops, M1/M2/M3 Macs | `qwen2.5-coder:7b`, `llama3:8b`, `deepseek-r1:8b` |
| 14B to 32B | 32 GB RAM, or a GPU with 12 GB+ | Workstations, Mac Pro/Max with 32 GB+ | `qwen2.5-coder:14b`, `deepseek-r1:32b` |

`brok setup llm` applies exactly this table. Override it any time:

```sh
brok setup llm --provider ollama --model llama3:8b
```

## Option 1: Ollama (recommended, terminal friendly)

| OS | Install |
|---|---|
| Windows | `winget install Ollama.Ollama` or the installer from [ollama.com](https://ollama.com) |
| macOS | `brew install ollama` or the app from [ollama.com](https://ollama.com) |
| Linux | `curl -fsSL https://ollama.com/install.sh \| sh` |

```sh
ollama pull qwen2.5-coder:7b     # or let `brok setup llm` do it
brok setup llm
```

## Option 2: LM Studio (graphical)

1. Download it from [lmstudio.ai](https://lmstudio.ai) (Windows `.exe`,
   macOS `.dmg` for Apple Silicon or Intel, Linux `.AppImage`:
   `chmod +x LM_Studio-*.AppImage && ./LM_Studio-*.AppImage`).
2. Search for `Qwen2.5-Coder-7B` (or the size from the table), pick a
   `Q4_K_M` or `Q5_K_M` file and download it.
3. Open the **Developer / Local Server** tab, load the model and press
   **Start Server** (it listens on `http://localhost:1234`).
4. Run `brok setup llm`: it finds the server and lets you pick the model.

## Hosted models

Prefer a hosted model for some questions? Export its key and choose it per
request; the providers are preconfigured in `brok.yml`:

```sh
export ANTHROPIC_API_KEY=...        # PowerShell: $env:ANTHROPIC_API_KEY = "..."
brok ask -p anthropic "explain the retry policy"
```

Keys are read from the environment, never stored in `brok.yml`, and secrets
found in retrieved code are redacted before any prompt leaves your machine.

## Using the same model elsewhere

- **Aider**: `aider --model ollama/qwen2.5-coder:7b` inside your git
  repository (`pip install aider-chat`, Python 3.10 to 3.12).
- **Coding agents** (Claude Code, Cursor, VS Code, Codex): `brok setup agents`
  gives them the Brokspace search tool and skills; see
  [integrations.md](integrations.md).

## Performance tips

- **Apple Silicon**: memory is unified, so a 16 GB Mac runs 7B/8B models fast.
- **NVIDIA / AMD GPUs**: enable full GPU offload in LM Studio or Ollama.
- **Storage**: keep models on an SSD; loading from a spinning disk is painfully slow.
- **Smaller is often better** for interactive use: a fast 7B beats a slow
  14B for everyday questions; switch per request with `-p`.
