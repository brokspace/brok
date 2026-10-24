---
name: to-tickets
description: Break a spec, plan or conversation into tracer-bullet tickets with explicit blocking edges.
disable-model-invocation: true
---

# To tickets

Split work into small vertical slices that each deliver something runnable,
and publish them so humans and agents can pick them up independently.

The tracker setup in `docs/agents/issue-tracker.md` should have been provided.
If it is missing, tell the user to run `/setup-brokspace`, or offer to write
the tickets to a local Markdown file instead.

## Slicing rules

- **Vertical, not horizontal.** Every ticket cuts through all the layers it
  needs (CLI, logic, storage, tests) and ends in a demonstrable behavior. "Add
  the database schema" is not a ticket; "store and list one bookmark" is.
- **Tracer bullet first.** Ticket 1 is the thinnest end-to-end path that
  proves the architecture. Later tickets widen it.
- **Small.** One focused session for one agent or person. If you cannot write
  the acceptance checks in five bullets or fewer, split again.
- **Explicit edges.** Each ticket names the tickets it is blocked by. No
  edges means it can start now. Avoid chains longer than necessary: parallel
  work is the point.

## Ticket shape

```markdown
## Goal
One sentence: the behavior that exists after this ticket.

## Context
Parent spec link, relevant paths (found with `brok search`), glossary terms.

## Acceptance
- [ ] ...

## Blocked by
#12, #13 (or "none")
```

## Steps

1. Read the source (spec issue, plan, or this conversation) and `GLOSSARY.md`.
2. Draft the full list as titles plus edges and show it to the user as a
   numbered tree. Wait for a yes or for corrections.
3. Publish in dependency order so blockers have numbers before they are
   referenced. On GitHub, also link each ticket as a sub-issue of the spec when
   the spec is an issue (`gh issue edit` or the web UI if the CLI cannot).
4. Label every ticket with its category and `needs-triage`, or
   `ready-for-agent` if the user says the brief is complete.
5. Print the list with links and mark which tickets can start immediately.
