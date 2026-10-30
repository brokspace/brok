# Getting started with Brokspace

From zero to a productive workspace in about 15 minutes. Each step is short;
skip what you do not need.

1. [Install brok](#1-install-brok)
2. [Get a local LLM (optional)](#2-get-a-local-llm-optional)
3. [Set up your first workspace](#3-set-up-your-first-workspace)
4. [Use it every day](#4-use-it-every-day)
5. [Connect your coding agent](#5-connect-your-coding-agent)
6. [Configure](#6-configure)
7. [Skills](#7-skills)
8. [Shards](#8-shards)
9. [Troubleshooting](#9-troubleshooting)
10. [Update brok](#10-update-brok)
11. [Uninstall](#11-uninstall)

---

## 1. Install brok

**macOS / Linux**

```sh
curl -fsSL https://raw.githubusercontent.com/brokspace/brok/main/scripts/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/brokspace/brok/main/scripts/install.ps1 | iex
```

Both install two programs, `brok` and `brok-agent`, and check their
checksums. Open a new terminal and check:

```sh
brok --version
```

**Company laptop blocks the script?** Download `brok-windows-x86_64.exe`
from the [brok release](https://github.com/brokspace/brok/releases/latest)
and `brok-agent-windows-x86_64.exe` from the
[agent release](https://github.com/brokspace/brok-agent/releases/latest),
rename them `brok.exe` and `brok-agent.exe`, put both in one folder and add
that folder to your `PATH`.

**Prefer Cargo?** `cargo install brok-cli brok-agent` (needs a C compiler and
`protoc`), or `cargo install brok-cli brok-agent --no-default-features` for a
lighter build with text search only.

---

## 2. Get a local LLM (optional)

Search works without any LLM. Answers (`brok ask`, `chat`, `review`,
`commit`) need one. A local model is free, private and works offline.

**Ollama** (recommended):

| OS | Install |
|---|---|
| Windows | `winget install Ollama.Ollama` |
| macOS | `brew install ollama` |
| Linux | `curl -fsSL https://ollama.com/install.sh \| sh` |

Pick the model for your machine:

| Your memory | Model | Download |
|---|---|---|
| 8 GB | `qwen2.5-coder:1.5b` | `ollama pull qwen2.5-coder:1.5b` |
| 16 GB (or a GPU with 6 GB+) | `qwen2.5-coder:7b` | `ollama pull qwen2.5-coder:7b` |
| 32 GB (or a GPU with 12 GB+) | `qwen2.5-coder:14b` | `ollama pull qwen2.5-coder:14b` |

Not sure? Skip the pull: `brok init` (next step) measures your machine and
offers to download the right one.

The 1.5b model is fine for questions (`ask`, `chat`); reviews and commit
messages need 7b or larger to be useful.

Prefer a graphical app? [LM Studio](https://lmstudio.ai) works too: download a
model, open **Developer / Local Server** and press **Start Server**. More in
[local-llm.md](local-llm.md).

---

## 3. Set up your first workspace

Go to your project (one repository, or a folder holding several) and run
(want a sample project first? clone
[example-nestjs](https://github.com/brokspace/example-nestjs)):

```sh
cd ~/code/my-project
brok init
```

It walks through six steps and asks before anything big:

| Step | What happens |
|---|---|
| 1. Workspace | Creates `.brokspace/` with a commented `brok.yml` |
| 2. Agent | Installs `brok-agent`, the helper that tracks changes and talks to the LLM |
| 3. Semantic search | Downloads the search model once (~140 MB, shared by all your projects) |
| 4. Local LLM | Finds Ollama / LM Studio and picks a model that fits your machine |
| 5. Index | Indexes your code (functions, classes and sections, not raw text) |
| 6. Coding agents | Connects Claude Code, Cursor, VS Code or Codex (skills + search tool) |

Want no questions? `brok init --yes` accepts every recommendation.
Then check the result:

```sh
brok doctor
```

Every line is either ✓, or a `!` with the exact command that fixes it.

> Your next project is faster: the search model and your LLM choice are
> remembered, so `brok init` there takes seconds.

---

## 4. Use it every day

```sh
brok search "where do we validate tokens"     # find code by meaning or by name
brok ask "how does the refresh flow work?"    # an answer from your LLM, with sources
brok chat                                     # a conversation with follow-up questions
brok review                                   # AI review of your uncommitted changes
git add -p && brok commit                     # commit message written from the staged diff
```

The first `ask`, `chat`, `review` or `commit` starts brok in the background.
From then on the index follows you: saved files, commits, pulls and branch
switches are picked up within seconds.

| You want to... | Run |
|---|---|
| Start / stop the background service | `brok start` / `brok stop` |
| Restart it after editing `brok.yml` | `brok restart` |
| See what it is doing | `brok logs -f` |
| See what is indexed | `brok status` |
| Review only staged changes, or a branch | `brok review --staged` / `brok review --base main` |
| Just print a commit message | `brok commit --print` |
| Use a different model for one question | `brok ask -p anthropic "..."` |
| Show whole results, or JSON for scripts | `brok search "..." --full` / `--json` |
| Rebuild the index from scratch | `brok index --rebuild` |
| Tab completion | `brok completions bash >> ~/.bashrc` (also zsh, fish, powershell) |

Inside `brok chat`: `/skill grill-me` applies a skill to the conversation,
`/clear` forgets it, `/exit` leaves.

---

## 5. Connect your coding agent

If you skipped step 6 of `brok init`:

```sh
brok setup agents
```

It detects your agents and, for each one, adds:

- a **search tool** (`brok_search`) through MCP, so the agent finds code by
  meaning before grepping;
- the **built-in skills**, such as `/ask-brok`, `/grill-me`, `/to-spec`,
  `tdd` and `code-review`.

| Agent | Files written |
|---|---|
| Claude Code | `.mcp.json`, `.claude/skills/` |
| Cursor | `.cursor/mcp.json`, `.agents/skills/` |
| VS Code / Copilot | `.vscode/mcp.json`, `.agents/skills/` |
| Codex | `.agents/skills/` (prints the MCP lines for `~/.codex/config.toml`) |

Commit these files and your whole team gets the same setup. In your agent,
start with `/ask-brok` and describe what you want to do.

---

## 6. Configure

Everything lives in **`.brokspace/brok.yml`**. Every key is optional and the
defaults are sensible. The settings people change most:

```yaml
llm:
  default: ollama                 # which provider `brok ask` uses
  providers:
    ollama:
      base_url: http://localhost:11434/v1
      model: qwen2.5-coder:7b
    anthropic:                    # hosted models: the key comes from an env var
      base_url: https://api.anthropic.com/v1
      model: claude-sonnet-5-5
      api_key_env: ANTHROPIC_API_KEY

embeddings:
  enabled: true                   # false = text search only, less memory

index:
  exclude:                        # .gitignore is always respected too
    - "**/node_modules/**"
    - "**/*.min.js"
    - "docs/archive/**"

shards:
  brok-agent:
    settings:
      review:                     # optional AI review of matching files on change
        enabled: false
        include: ["src/api/**"]
```

Easier ways to change things:

| Change | Command |
|---|---|
| Switch the local LLM or model | `brok setup llm` (or `brok setup llm --provider ollama --model llama3:8b`) |
| Connect more coding agents | `brok setup agents` |
| Share the index (Git LFS, S3, R2, MinIO) | `brok setup sync` |

After editing `brok.yml`, run `brok restart`. Every key is explained in
[configuration.md](configuration.md).

**API keys** are never written to files: export them in your shell
(`export ANTHROPIC_API_KEY=...`, or `$env:ANTHROPIC_API_KEY = "..."` in
PowerShell) and Brokspace reads them when needed. Secrets found in your code
are removed from the context before anything is sent to an LLM.

---

## 7. Skills

A skill is a small, reusable workflow for AI: a `SKILL.md` file with
instructions. Brokspace ships 20 of them.

```sh
brok skill list                                    # what is available
brok skill show tdd                                # read one
brok skill run code-review "the session handling"  # run one on your local LLM
```

**User-invoked** skills are started by you (type `/name` in your agent, e.g.
`/grill-me` before a big change). **Model-invoked** skills are picked by the
agent when the task fits (e.g. `tdd`, `code-review`).

Write your own:

```sh
brok skill new release-notes          # creates .brokspace/skills/release-notes/SKILL.md
brok skill export                     # refresh your coding agents with it
```

A workspace skill with the same name as a built-in one replaces it, so you
can tailor any built-in to your team. More in [skills.md](skills.md).

---

## 8. Shards

Shards are small programs that react to what happens in your workspace (a
file changed, a question was asked) and can publish their own events. The
agent you installed is one.

```sh
brok shard list                                             # what is installed
brok shard install https://github.com/someone/some-shard    # shows its permissions before installing
brok shard new my-shard                                     # start your own (Rust project)
brok shard new my-skills --skills-only                      # or just share skills, no code
brok restart                                                # start new shards
brok bus tail                                               # watch events live while developing
```

Every shard lists the permissions it wants in `shard.json`, and you approve
them when installing. Guide: [shards.md](shards.md).

---

## 9. Troubleshooting

Start with `brok doctor`: it checks everything and prints the fix.

| Symptom | Fix |
|---|---|
| `no answer from the agent` | Start your LLM (open Ollama / LM Studio), then `brok doctor` |
| `ollama ... is not installed` | `ollama pull <model>` as shown by `brok doctor` |
| Answers take minutes | The model runs on your CPU. Normal for 7b on a laptop without a GPU (a review can take 3 to 5 minutes); for faster answers try `ollama pull qwen2.5-coder:3b` and `brok setup llm --model qwen2.5-coder:3b` |
| Search misses recent code | `brok status`; if the daemon is stopped, `brok start` catches up |
| `another brok process is writing the index` | `brok` is running in the background and already indexes for you; skip `brok index`, or `brok stop` first |
| Results come back as text only | The search model is missing: `brok model download` |
| Agent does not show the brok tools | `brok` must be on `PATH`; run `brok setup agents` and restart the agent |
| Odd behavior after an upgrade | `brok restart`; if it persists, `brok index --rebuild` |
| Windows: errors about long paths | Enable long paths (`brok doctor` shows the command) or use a shorter folder |

Still stuck? Run `brok logs -n 100` and ask in
[Discussions](https://github.com/brokspace/brok/discussions), including the
output of `brok doctor`.

---

## 10. Update brok

```sh
brok upgrade --check    # is there a new version, and what changed?
brok upgrade            # shows what changed, asks, then upgrades
```

`brok upgrade` downloads the new `brok` and `brok-agent`, checks them
against the release checksums, swaps them in and restarts brok in every
workspace where it was running (each workspace's agent is refreshed on
that restart). `brok doctor` tells you when a new version is out.

Then run `brok skill export` in your workspaces so your coding agents get
the updated built-in skills.

Installed with Cargo? Upgrade with `cargo install brok-cli brok-agent`.
Coming from brok 0.2.1 or older (no `upgrade` command yet)? Run the install
command from [step 1](#1-install-brok) again, once.

---

## 11. Uninstall

```sh
brok stop                    # in each workspace where it runs
```

Then delete:

- the `.brokspace/` folder of each workspace (plus `.agents/skills`,
  `.claude/skills` and the MCP files if you no longer want them);
- the shared model cache: `~/.cache/brokspace` (Linux),
  `~/Library/Caches/brokspace` (macOS), `%LOCALAPPDATA%\brokspace` (Windows);
- your defaults: `~/.config/brokspace` (Linux),
  `~/Library/Application Support/brokspace` (macOS), `%APPDATA%\brokspace`
  (Windows);
- the `brok` and `brok-agent` programs (`~/.local/bin`, or
  `%LOCALAPPDATA%\Programs\brok` on Windows; `cargo uninstall brok-cli brok-agent`
  if you used Cargo).
