---
name: domain-modeling
description: "Build and sharpen a project's shared language and design decisions: challenge terms against GLOSSARY.md, test them with edge cases, and record ADRs. Use when a term is ambiguous or overloaded, when naming a new concept, when two people mean different things by one word, or when a hard-to-reverse decision is made."
---

# Domain modeling

A shared language makes agents and people shorter and more precise: one
agreed word replaces a paragraph, and code names follow the words. This skill
keeps that language honest. Merely reading `GLOSSARY.md` for vocabulary does
not need this skill; actively changing it does.

## Challenge terms

Whenever a term comes up that is new, vague, or used two ways:

1. Look it up in `GLOSSARY.md` and in the code (`brok search "<term>"`).
2. If it exists, check the usage matches the definition. If not, point out
   the conflict and ask which meaning wins.
3. If it is new, propose a definition of one or two sentences, plus words to
   avoid.
4. Stress-test the definition with two or three edge cases: "Is a deleted
   item still a Foo?", "Can a Foo belong to two Bars?". Adjust until the
   answers are obvious from the definition.

Update `GLOSSARY.md` as soon as a term is agreed, following
[GLOSSARY-FORMAT.md](GLOSSARY-FORMAT.md). Then use exactly that word in
questions, specs and code.

## Record decisions

Write an ADR when a decision is any of: hard to reverse, surprising to a
newcomer, or the outcome of a real trade-off between options. Do not write
ADRs for obvious choices.

- Location: `docs/adr/NNNN-short-title.md` (next free number), unless the
  repo's docs say otherwise.
- Format: [ADR-FORMAT.md](ADR-FORMAT.md).
- Never edit an accepted ADR's decision; supersede it with a new one and link
  both ways.

## Watch for

- **Synonyms**: two words for one thing. Pick one, list the other under
  "Avoid".
- **Homonyms**: one word for two things. Split it into two terms.
- **Code drift**: identifiers that no longer match the glossary. Note them;
  renaming is a separate, deliberate change.
