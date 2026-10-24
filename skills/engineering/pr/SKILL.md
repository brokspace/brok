---
name: pr
description: "Write a pull request description that makes the change obvious, proves it works and states its risk. Use when opening or updating a pull request, when the user asks for a PR body or summary of a branch, or at the end of implementation."
---

# Pull request

A reviewer should understand what changed, believe that it works, and know
how dangerous it is to merge, in under a minute.

## Gather

- `git log --oneline <base>..HEAD` and `git diff --stat <base>..HEAD`.
- The issue or spec it resolves.
- What you actually ran to verify it (commands and their results).

## Shape

Use [TEMPLATE.md](TEMPLATE.md). The rules behind it:

1. **Summary as the smallest visual that makes it clear.** A before/after
   table, a three-line command transcript, a tiny diagram, or two sentences.
   Not a list of every file touched.
2. **Evidence.** Real output, test names, screenshots for UI, timings for
   performance. "Tests pass" alone is not evidence; say which tests and what
   they cover.
3. **Risk.**
   - Door: **two-way** (easy to revert: internal refactor, new optional flag)
     or **one-way** (hard to undo: data migrations, wire protocol, manifest or
     config formats, public APIs, published releases).
   - Blast radius: who or what breaks if this is wrong, and how you would
     notice.
4. **Review guide** for anything over about 300 lines: where to start and
   what can be skimmed.

## Title

Conventional Commits, imperative, under 72 characters:
`feat(skills): run skills on the local model`. Reference the issue in the
body with `Closes #42`.

## Publish

If asked to open the PR: `gh pr create --title "<title>" --body-file <file>`
(or the GitLab equivalent from `docs/agents/issue-tracker.md`). Do not push or
open PRs unless the user asked for it.
