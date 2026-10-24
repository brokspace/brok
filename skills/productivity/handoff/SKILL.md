---
name: handoff
description: Compact this session into a handoff document so another agent or person can continue the work.
disable-model-invocation: true
---

# Handoff

Write everything the next agent needs to continue without reading this
conversation, and nothing it does not need.

## Where

- Default: `.scratch/handoffs/YYYY-MM-DD-<slug>.md` in the repository (make
  sure `.scratch/` is in `.gitignore`; add it if not, and say so).
- If the user prefers not to touch the repo: the OS temp directory
  (`$TMPDIR`, `/tmp`, or `%TEMP%`).

## What

```markdown
# Handoff: <goal in one line>

## Goal
What we are trying to achieve and how we will know it is done.

## State
- Branch and last commit: `git branch --show-current`, `git log -1 --oneline`
- Done: ...
- In progress: ... (file:line where work stopped)
- Uncommitted changes: summary of `git status --short`

## Decisions
Numbered, one line each, with the reason. Link issues, specs and ADRs.

## Open questions
What is unresolved and who can answer it.

## Next steps
1. The very next action, specific enough to start immediately.
2. ...

## Commands
Exact commands that build, test and reproduce things in this repo, e.g.
`cargo test -p brok-index`, `brok search "..."`, `brok run`.

## Gotchas
Traps found during the session (flaky test, slow step, misleading name).
```

## Rules

- Facts over narrative. Paths with line numbers, commands that actually ran.
- Include failed approaches only if the next agent would otherwise retry them.
- Keep it under two screens. Link instead of pasting long output.
- Print the file path at the end, plus a one-line prompt the user can paste
  into a new session: "Read <path> and continue from Next steps."
