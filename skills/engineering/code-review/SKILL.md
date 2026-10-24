---
name: code-review
description: "Review a diff on two axes, coding standards and fidelity to the spec, and report severity-ranked findings with path:line. Use when the user asks for a review, before committing or opening a pull request, or when another skill closes out implementation."
---

# Code review

Review the change since a fixed point (default: the merge base with the main
branch, `git diff $(git merge-base HEAD origin/main)`; or the commit the
session started from). Two independent passes, so neither biases the other.

## Gather

- The diff, plus enough surrounding code to understand each hunk. Use
  `brok search` (or `brok_search`) to find callers and similar code the diff
  should be consistent with.
- The originating spec, ticket or request.
- The repo's standards: `AGENTS.md`, `CONTRIBUTING.md`, lint config, ADRs in
  the touched area, `GLOSSARY.md`.

## Pass 1: standards

Ignore the spec. Judge the code as code:

- Correctness risks: unhandled errors, panics on user input, races, resource
  leaks, off-by-one, platform assumptions (paths, line endings).
- Security: input validation, secrets in code or logs, injection, permissions.
- Design: names that disagree with the glossary, logic in the wrong module,
  duplication of an existing helper, shallow wrappers, long functions.
- Tests: behavior covered at public seams, tautological or implementation
  coupled tests, missing regression test for a bug fix.
- Project rules from the docs above.

## Pass 2: spec fidelity

Ignore style. Compare behavior to the request:

- Every acceptance item: implemented, partially, or missing?
- Anything implemented that nobody asked for?
- Edge cases the spec names or implies.
- User-visible text, flags, config keys and docs match the spec.

## Report

Merge both passes into one list, most severe first:

```
[must-fix | should-fix | nit] path/to/file.rs:42  <axis: standards|spec>
  What is wrong, why it matters, and the concrete fix.
```

End with: a one-line verdict (ready, ready after must-fixes, needs rework),
and what you did not check (for example, "did not run the Windows build").
Report only findings you can point to in the code; say "no findings" for an
axis rather than inventing some.
