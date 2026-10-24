---
name: implement
description: Build the work described by a spec or tickets, test-first at agreed seams, closing with a review and a commit.
disable-model-invocation: true
---

# Implement

Turn a spec, a ticket or a clear request into working, tested, reviewed code.

## 1. Load the work

- Read the source: the issue (via the command in `docs/agents/issue-tracker.md`),
  the file, or the conversation. If it is a spec with several tickets, work one
  ticket at a time, in dependency order, starting with any that are unblocked.
- Read `GLOSSARY.md` and the ADRs for the area. Use the glossary terms for
  every new name.
- Locate the code. Call the Skill tool with "brok-context" and search the
  index for the modules, symbols and similar features involved. List the
  files you expect to change.
- Restate the goal and acceptance checks in your own words. If anything is
  ambiguous, ask now, once, with a recommended answer for each question.

## 2. Agree the seams

Propose the public interfaces you will test through (a function, a CLI
command, an HTTP route, a bus topic) with one line each on what that seam
catches and what it misses. Wait for the user to confirm. No test is written
at an unconfirmed seam.

## 3. Build test-first

Call the Skill tool with "tdd" and follow it: one failing test, the least
code that passes it, repeat, one vertical slice at a time. Run the project's
checks after every slice (for a Rust repo: `cargo test`, then
`cargo clippy --all-targets -- -D warnings`). Stop and report if a check fails
in a way you cannot explain after two attempts.

## 4. Review

Call the Skill tool with "code-review" on the diff since the commit you
started from. Fix every finding marked must-fix. Refactor here, not during
the red-green loop. Re-run the checks.

## 5. Commit and hand back

- Commit with a Conventional Commits message that references the issue
  (`feat(index): lazy submodule partitions (#42)`). Do not push unless asked.
- If the user wants a pull request, call the Skill tool with "pr" for the
  description.
- Report: what changed (paths), the seams tested, checks run with results,
  anything deferred. Update the ticket (comment or checkbox) if the tracker
  allows it.
