---
name: commit-message
description: "Write a Conventional Commits message for a staged change. Use when the user asks for a commit message, wants to commit their work, or runs brok commit."
---

# Commit message

Turn a staged diff into one commit message that a reviewer understands
without opening the diff.

## Steps

1. Read the diff (`git diff --cached`; `brok commit` passes it to you) and
   decide what the change does for a user or a maintainer. Ignore formatting
   noise.
2. Pick the type: `feat` (new behavior), `fix` (a bug), `docs`, `refactor`
   (no behavior change), `perf`, `test`, `build`, `ci` or `chore`.
3. Pick a scope from the area touched, using the project's words (see
   `GLOSSARY.md` when it exists): `feat(index): ...`. Omit it when the
   change spans the whole project.
4. Write the subject: imperative mood ("add", not "added"), lowercase after
   the colon, no final period, 72 characters or fewer.
5. Add a body only when the why is not obvious from the subject: what was
   wrong or missing, and what changes now. Wrap at 72 characters. Never
   narrate the code line by line.
6. A breaking change gets `!` after the type or scope and a footer
   `BREAKING CHANGE: <what users must do>`.

## Output

Return only the message: subject, optional blank line and body, optional
footers. No code fences, no quotes, no commentary before or after.

## Example

```
fix(bus): route replies only to the requester

Replies were broadcast to every subscriber of the reply topic, so other
shards could read answers meant for someone else.
```
