---
name: grill-with-docs
description: Get interviewed about a change until it is fully specified, while building the repo's GLOSSARY.md and ADRs.
disable-model-invocation: true
---

# Grill with docs

An interview that aligns you and the agent on a change before any code is
written, and leaves the project's shared language better than it found it.

## Setup

1. Read `GLOSSARY.md` and the ADRs in `docs/adr/` (or wherever
   `docs/agents/issue-tracker.md` says docs live). If they do not exist, they
   will be created as decisions land.
2. Explore the code the change touches. Call the Skill tool with
   "brok-context" for how to search the Brokspace index. Summarize what exists
   in five lines or fewer before the first question.

## The session

Call the Skill tool twice, for "grilling" and "domain-modeling". Then run the
interview with both disciplines active:

- `grilling` drives: one question at a time, each with your recommended
  answer, until every branch of the design is resolved.
- `domain-modeling` listens: whenever a term is fuzzy, overloaded or new,
  challenge it against the glossary and fix the language before moving on.

Write as you go, not at the end:

- A term gets agreed: add or update it in `GLOSSARY.md` immediately.
- A decision is hard to reverse, surprising, or the result of a real
  trade-off: write an ADR (`docs/adr/NNNN-short-title.md`) and mention it.
- Use the agreed terms from then on, in questions and in code names.

## Finish

When nothing is left open, print:

1. **Decisions**: a numbered list, each one line.
2. **Docs updated**: glossary terms added or changed, ADRs written.
3. **Open risks**: anything deferred on purpose.
4. **Next**: suggest `/to-spec` to publish it, or `/implement` if the change
   is small enough for one sitting.
