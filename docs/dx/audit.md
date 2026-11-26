# DX 1.0: current state and usability gaps

Discovery only, 2026-11-26. Observed with the released brok 0.10.0 and brok-agent 0.5.1 on one Linux
x86_64 machine (20 threads, 31 GB RAM, fast network) in a fresh `HOME`, on a project of 147 files
(this repository's `crates/` and `docs/`). VS Code behaviour comes from reading `vscode` at `staging`
and its rendered screenshots, **not** from running the editor.

## What already works well

- `install.sh` took 14 s, verified checksums and installed both programs.
- `brok init` is one command in six labelled steps; `brok doctor` gives a fix per warning.
- Offline setup finishes with text search and says why semantic search is off.
- A missing embedding model degrades search to text with a clear warning.
- Search takes about 0.3 s without the daemon and about 20 ms with it.
- A broken `brok.yml` is reported with file and line.
- The extension verifies its download, never replaces an existing install, has cancellable progress,
  a first run checklist, English and Portuguese, and tested accessibility rules.

## Problems, most important first

Observed before any fix. P1 is addressed by brokspace/brok#195.

| # | Problem | Evidence | Who is hit |
|---|---|---|---|
| P1 | **Private memories and metrics can be committed.** `.brokspace/.gitignore` does not ignore `memory/` or `metrics/`, while `docs/memory.md` says private memories are "Never committed". | Reproduced: after `brok memory add`, `git status` lists the memory files and `metrics/*.jsonl`, which holds error messages. Template: `crates/brok-core/src/init.rs`. | Anyone who runs `git add .`; saved chats are memories too |
| P2 | **The extension's download installs `brok` without `brok-agent`.** No answers, review, commit messages or live indexing. | `src/core/installer.ts` downloads only `brok`. With only `brok`, setup warns, `doctor` lists the agent, and the suggested fix is a manual install. | VS Code users without a prior install |
| P3 | **`brok ask` without the agent waits silently.** | Killed after 90 s with no message; `FIRST_TOKEN_TIMEOUT` is 600 s. | Same users |
| P4 | **First search waits for all embeddings.** | 152 s for `brok init --yes`, 135 s of it indexing. The same size of project indexes as text only in about 0.1 s (measured on 0.9.0). | Every first run |
| P5 | **Cancelling the first index loses all of it** and leaves a confusing status. | Ctrl+C after 45 s: `status` shows `0 files 512 chunks`; the next run takes the full 132 s again. | Large projects |
| P6 | **Setup writes into the repository without saying so in the editor.** | `brok init --yes` (what "Set up this folder" runs) creates `.agents/skills` and `.vscode/mcp.json`, while the checklist says "Nothing in your code is changed". Without a terminal, `brok init` accepts every step. | VS Code and scripted users |
| P7 | **An unreachable model shows a raw error chain** with no next step. | `brok ask` with Ollama stopped: `error sending request for url (...) Connection refused (os error 111)`, repeated. | Anyone without a running model |
| P8 | `brok review` in a repository with no commit shows git's own error. | `fatal: ambiguous argument 'HEAD'`. | New projects |
| P9 | The narrow sidebar cuts the workspace name and the model pill. | `docs/images/first-run-dark.png`: `sh...`, `Index-only · Conne...` (sample data). | Narrow sidebars |
| P10 | Package managers are behind. | APT serves brok 0.8.0 and brok-agent 0.5.0. | APT and Homebrew users |

## Not examined (do not read as working)

Real VS Code sessions, activation time and UI responsiveness; Windows and macOS journeys; answers from
a real model; hosted provider errors; MCP failures inside an agent; offline installation of the
binaries; interrupted conversations; screen readers; idle CPU in this session; multi-repository setups.

## Current versus target journeys

| Moment | Today | Target |
|---|---|---|
| Install in VS Code | Downloads `brok` only | Downloads both, or says what is missing and fixes it |
| First search | After the whole semantic index (152 s here) | Text results within seconds, then hybrid |
| Cancel indexing | Starts over | Keeps progress, resumes |
| Files written to the repo | Implicit | Listed and confirmed |
| No model | Raw connection error in the CLI | One sentence, one action, index answer still available |
| Private data in git | Committable | Ignored by default, existing workspaces repaired |
