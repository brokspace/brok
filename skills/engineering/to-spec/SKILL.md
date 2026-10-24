---
name: to-spec
description: Turn the current conversation into a spec and publish it to the issue tracker.
disable-model-invocation: true
---

# To spec

Synthesize what has already been discussed into a spec and publish it. Do
not interview; the alignment already happened (or the user chose to skip
it). Only ask when something required below is genuinely missing.

The issue tracker and label strings should have been configured in
`docs/agents/issue-tracker.md` and `docs/agents/triage-labels.md`. If they are
missing, tell the user to run `/setup-brokspace` and stop.

## Before writing

- Read `GLOSSARY.md` and use its terms exactly.
- Identify the modules the change touches. Search the index for each one
  (`brok search "<module or concept>"`, or the `brok_search` tool) and name
  them by path. Ask the user to confirm the list if you are unsure; a wrong
  module list makes a wrong spec.

## Spec shape

```markdown
## Problem
Who is affected, what hurts today, why it matters now. Two to five sentences.

## Outcome
What is true when this is done, observable from outside the code.

## Scope
- In: ...
- Out: ... (explicitly deferred)

## Design
Modules touched (with paths), interfaces added or changed, data shapes,
decisions and the ADRs that record them.

## Acceptance
- [ ] Testable statements, each verifiable by a person or a test.

## Risks and open questions
```

Keep it short. A spec that does not fit on one screen usually hides two specs.

## Publish

1. Create the issue with the tracker command from `docs/agents/issue-tracker.md`,
   title in the imperative ("Index git submodules lazily").
2. Apply the `enhancement` (or `bug`) category label and the `needs-triage`
   state label, using the repo's label strings.
3. Print the issue link or file path, and suggest `/to-tickets` when the
   acceptance list has more than three independent items.
