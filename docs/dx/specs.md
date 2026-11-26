# DX 1.0: principles, specifications and wireframes

## Principles

1. **Value before configuration.** A search works before any model, account or choice.
2. **Say what changes.** Files written, downloads and anything leaving the machine are named first.
3. **One next action.** Every failure says what happened and offers a single way forward.
4. **Progress is kept.** Long work can be cancelled and resumes.
5. **One source of truth.** `brok.yml` and the CLI; the editor renders what brok reports.
6. **Private by default.** What is personal is never committable by default.

## Functional specifications

| Id | Behaviour | Acceptance |
|---|---|---|
| S1 Ignore rules | `memory/` and `metrics/` are in `.brokspace/.gitignore`; existing workspaces gain the lines at the next brok command, keeping the user's own lines; `doctor` warns if such files are already tracked. | `git status` after using memory shows none of them; an edited ignore file keeps its content |
| S2 Complete install | The extension installs `brok` and `brok-agent` of compatible versions, each verified; a failure leaves the previous install. | From an empty machine, ask works once a model is configured |
| S3 Fail fast | `ask`, `chat`, `review`, `commit` check that the agent is connected and the model reachable; otherwise they stop within seconds with the fix. | No wait longer than 10 s before the message; exit code and JSON error stable |
| S4 Text first | Setup saves the keyword index first and says search is ready; vectors are added in saved batches. | First search under 10 s on the reference project; Ctrl+C keeps finished batches |
| S5 Consent | Setup in the editor lists the repository files it would write and asks; `brok init` without a terminal and without `--yes` writes none of them. | Checklist text matches what happens |
| S6 Readiness | `status --json` reports agent, index and model readiness and one suggested action; the editor shows it. | The editor needs no parsing of error text |

## Wireframes (low fidelity)

Sidebar widths about 300 px. `[ ]` button, `( )` choice.

### 1. First run
```text
+------------------------------------------+
| Getting started                 2 of 5   |
| [x] How your data is handled             |
| [x] brok and agent installed             |
| [>] Set up this folder                   |
|     Creates .brokspace/ (ignored files   |
|     stay private) and indexes.           |
|     Also write agent files?              |
|     [x] .vscode/mcp.json                 |
|     [x] .agents/skills (20)              |
|     [ Set up ]                           |
| [ ] Search or ask                        |
| ( ) Connect a model        Optional      |
+------------------------------------------+
```

### 2. Workspace overview and initialization
```text
| Setting up payments-api                  |
| Text search      ready   [ Search now ]  |
| Meaning search   42%  =====-----         |
|   620 of 1500 chunks      [ Pause ]      |
| Agent            running                 |
| Model            not set [ Connect ]     |
```

### 3 and 4. Chat, search results and sources
```text
| Chat | History | Settings                 |
| payments-api   [ Index only ]    Tools v  |
| ! Meaning search 42%. Results are text.   |
|              where are tokens validated? >|
| Validated in AuthGuard ...                |
|   auth.guard.ts:22-41 canActivate         |
|   jwt.strategy.ts:14-38 validate          |
|   index answer   Copy  Retry              |
| [ Ask, / commands, @ context           ]  |
| [+ Context]                      [Send]   |
```

### 5. Conversation history
```text
| Search history [______________]          |
| Today                                     |
|  Token rotation          3 turns   ...    |
|  Review of staged diff   1 turn    ...    |
| Resume / Rename / Delete                  |
```

### 6. Context attachments
```text
| (file x) auth.guard.ts:22-41              |
| (diff x) working tree, 2 files            |
| (!)  .env not attached: holds credentials |
```

### 7. Activity and approval
```text
| v Activity                                |
|   read index     8 passages      0.1 s    |
|   call model     qwen via ollama 12 s     |
|   (approvals wait for brok#152; no write  |
|    tool exists today)                     |
```

### 8. Memory and skills
```text
| Memory   [ Remember ]   private | shared  |
|  decision  Cache profiles 5 min           |
| Skills   /code-review  /tdd  ... [ Run ]  |
```

### 9. Settings and providers
```text
| Model                                     |
|  (o) ollama  localhost:11434  not running |
|      [ How to start ] [ Test ]            |
|  ( ) hosted  key from $OPENAI_API_KEY     |
|      Your question and passages are sent. |
```

### 10. Health and recovery
```text
| Health                       [ Re-check ] |
|  ok    brok 0.10.0                        |
|  fix   agent missing     [ Install ]      |
|  fix   model unreachable [ Switch ]       |
|  [ Doctor output ] [ Logs ] [ Report ]    |
```

## Visual direction

Keep `vscode/docs/design-system.md`: `--vscode-*` colours, the forge accent, Codicons, the existing
`bk-` components, keyboard, screen reader, reduced motion and high contrast rules, EN and pt-BR. No new
theme and no new component unless a wireframe cannot be built with the existing ones. Refine visuals
only after these flows are agreed.

## Technical recommendations

- S1: extend `GITIGNORE` in `brok-core` and add an idempotent "ensure these lines" step on workspace
  open for writing commands. No config change.
- S2: reuse `installer.ts`; add the agent asset from the `brok-agent` releases and place it next to
  `brok`. Coordinate the compatible version range with brok#46.
- S3: reuse the `daemon:status` `starting` and `identities` fields added for brok#179; add a quick
  provider reachability check already used by `doctor`.
- S4: save the keyword index and manifest per batch in `Index::process`; mark vectors pending in the
  manifest. This changes index state, so it needs a design record first.
- S5: add a flag to `brok init` to skip repository files (it has `--no-integrations`) and have the
  extension ask.
- S6: additive fields in `status --json`. No `brok-sdk` change is needed for S1 to S6.

## Metrics

Baselines from this discovery or earlier measurements on the same machine. **Targets are proposals**
to confirm after measuring on macOS, Windows and a laptop.

| Metric | Baseline | Proposed target |
|---|---|---|
| Install (download and verify) | 14 s | keep |
| First successful search, 147 files | about 166 s after install starts | under 30 s |
| Full semantic index, 1504 chunks | 135 s (20 threads) | do not regress; runs in background |
| Incremental index, one file | 0.9 s (0.9.0) | under 2 s |
| Search latency | 20 ms daemon, 300 ms cold | keep |
| Vector search, 100 000 chunks | 9 ms (brok#192) | keep |
| Peak memory, first index | 635 MB | under 700 MB |
| Time to an explained failure without a model or agent | up to 600 s | under 10 s |
| Extension activation, UI response, idle CPU | not measured | measure in M0 |

Reuse `brok-bench` for the core numbers; add an extension activation measurement.
