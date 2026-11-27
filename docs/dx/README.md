# DX 1.0 proposal: powerful inside, effortless outside

Status: **priority roadmap**, taken ahead of the orchestrator and PR Intelligence work (2026-11-26).
The privacy fix is brokspace/brok#195; the other changes are not implemented yet. Documents: [audit](audit.md) · [principles, specifications, wireframes, metrics](specs.md) ·
[diagrams](diagrams/).

## Executive summary

The pieces for a good first experience exist: a verified installer, a guided `brok init`, `doctor`, an
editor checklist, and search that needs no model. A first-time developer is still stopped or surprised
in ways that are cheap to fix: private data can be committed, the editor installs half of the product,
failures wait or print raw errors, and the first search waits minutes for work that could happen later.

## Product and architecture assessment

One CLI owns configuration and the index; the editor and coding agents go through it
([current](diagrams/architecture-current.mmd), [proposed](diagrams/architecture-dx-proposed.mmd)).
That is the right shape and DX 1.0 keeps it: no new service, store, configuration database or protocol.
Orchestration (brok#163) and PR Intelligence (brok#178) stay out of scope and unchanged.

## Roadmap

Each change is one small pull request against `staging`. Definition of done for every milestone: acceptance met on Linux, macOS and Windows, tests in
CI, changelog entry, docs updated, no regression of the baselines. Order across repositories:
[dependency map](diagrams/dependency-map.mmd).

### M0: specifications (this set)
- **Scope:** audit, flows, wireframes, metrics; measure what is unmeasured (macOS, Windows, a real
  editor session, activation time, a real model).
- **Repos:** brok, vscode. **Issues:** vscode#40 to #48 (review what shipped).
- **Acceptance:** maintainer agrees on problems, order and targets. **Tests:** none. **Risks:** none.

### M1: a safe and complete first run
| PR | Repo | Issue | Change | Tests |
|---|---|---|---|---|
| 1 | brok | brok#195 (privacy, urgent) | S1 ignore rules and repair | unit and CLI test with real git |
| 2 | vscode | vscode#19 | S2 install brok and the agent | installer unit tests for both assets, failure leaves previous |
| 3 | brok | brok#197 | S3 fail fast with a fix (agent, model) | CLI tests with no agent and a closed port |
| 4 | brok, vscode | brok#198, vscode#46 | S5 consent for repository files; checklist text | CLI no-terminal test; webview tests |
| 5 | brok | brok#199 | Clear message for a repository without commits (P8) | CLI test |
- **Depends on:** brok#46 for the version range of PR 2. **Security:** PR 1 reduces exposure but cannot
  remove files already committed, so `doctor` must say how; PR 2 adds a download from a second
  repository under the same checksum rule. **Compatibility:** additive; existing ignore files keep
  user lines.
- **Acceptance:** from an empty machine to a search and an explained answer with no dead end.

### M2: search in seconds, work that is never lost
- **PRs:** design record, then saved batches and text-first indexing (S4), then readiness in
  `status --json` and the editor status line (S6).
- **Repos:** brok, vscode. **Issues:** brok#72, vscode#40, #41, brok#200 (S4), brok#201 (S6).
- **Depends on:** M1. **Risks:** index state changes; an interrupted write must never corrupt; older
  brok must keep rebuilding safely. **Tests:** kill during indexing, resume, mixed old and new index.
- **Security and compatibility:** nothing new leaves the machine; the manifest change needs a version
  step like ADR 0017.
- **Acceptance:** the first search and cancellation targets in specs.md.

### M3: daily productivity
- **Scope:** confirm and polish what shipped: composer, context, history, activity; narrow layout (P9).
- **Repos:** vscode, brok. **Issues:** vscode#42 to #45, #47, brok#152 (only if approvals are wanted),
  brok-agent#24 (only if the history cap proves painful).
- **Depends on:** the manual review of M0. **Tests:** webview and integration tests, screen reader and
  keyboard pass. **Security:** approvals need a write tool that does not exist; do not fake them.
  **Compatibility:** `engines.vscode` stays at 1.90. **Acceptance:** each shipped vscode issue is
  confirmed in a real editor and closed or reopened with what failed.

### M4: documentation and adoption
- **Issues:** website#58, vscode#48, brok#111, brok#42 (APT, Homebrew), brok#44 (signing), brok#43.
- **Scope:** a five minute quickstart that ends in a search, short recording, troubleshooting by
  symptom, contributor path. **Tests:** quickstart commands run in CI where practical; site build and
  link check. **Security and compatibility:** signed or attested binaries (brok#44); published commands
  match the released version. **Depends on:** M1. **Acceptance:** a person new to the project follows it
  unaided.

## Existing issue mapping

| Issue | State seen | Use |
|---|---|---|
| vscode#19 | open; download exists, without the agent | M1 |
| vscode#40 to #44, #46 to #48 | shipped in 0.4.0 and 0.5.x, open pending manual review | M0, M3 |
| vscode#45 | partly shipped; blocked on brok#152 | M3 |
| brok#72 | scan, batch size and doctor warning done; profiles on hold | M2 |
| brok#192 | **fixed** 2026-11-26, in the pending 0.11.0 release | done |
| brok#46 | version compatibility open | M1 dependency |
| brok#42, #43, #44 | packaging and signing | M4 |
| website#58, brok#111 | adoption docs | M4 |

## Risks

- **Privacy:** until the release with brok#195, tell users to check `git status` before committing `.brokspace/`.
- **Compatibility:** brok, brok-agent and the extension version independently; M1 PR 2 must pin a range.
- **Performance:** text-first indexing must not make the total index time or memory worse.
- **Scope:** the unmeasured platforms may reorder M1 to M4.

## Recommended first milestone, and the smallest complete improvement

**Implement M1** (the privacy fix, brok#195, first). The smallest complete improvement is: *a first run that never leaks, never stops at
half a product and never leaves the developer waiting*: PR 1 (ignore rules), PR 2 (install the agent
too) and PR 3 (fail fast with one next action). PR 1 should go first and not wait for the rest of the
review. Text-first indexing (M2) is the largest felt gain, but it changes index state and deserves its
design record.
