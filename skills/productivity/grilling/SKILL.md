---
name: grilling
description: "Interview the user relentlessly about a plan, design or decision until every branch is resolved. Use when requirements are vague, when a plan has open branches, before writing a spec, or when another skill needs questions answered before it can act."
---

# Grilling

Resolve every open branch of a plan by asking the right questions, one at a
time, until nothing important is left to guess.

## Before asking

- Read what already exists: the conversation, linked issues, `GLOSSARY.md`,
  and the relevant code (search the Brokspace index with `brok search` when it
  is available). Never ask what you can find out yourself.
- Build a mental tree of decisions: each unresolved choice is a branch, and
  answering one may open others.

## Asking

- **One question per message.** Wait for the answer before the next.
- **Always recommend.** Each question ends with your recommended answer and a
  one-line reason, so the user can reply "yes".
- Offer options when the space is small: `A) ... B) ... C) ...`.
- Ask the questions with the most downstream impact first (data shape before
  button labels).
- Prefer concrete scenarios over abstract preferences: "A user deletes a
  workspace while `brok run` is indexing it. What should happen?"
- If the user says "you decide", decide, state it as a decision, and move on.
- If an answer contradicts an earlier one, point out the conflict and ask
  which wins.

## Stopping

Stop when every branch has a decision, or the user calls time. Then print:

1. **Decisions**: numbered, one line each.
2. **Deferred**: questions left open on purpose, with who decides.
3. **Assumptions**: anything you assumed without asking.

Hand these back to the calling skill or the user. Do not start implementing.
