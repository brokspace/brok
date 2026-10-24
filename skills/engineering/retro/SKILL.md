---
name: retro
description: After a session, suggest improvements to the agent's environment (docs, checks, skills, tooling), most severe first.
disable-model-invocation: true
---

# Retro

Look back at the session that just happened and find what in the
environment made the agent slower, wrong or noisy. The goal is fixes to the
repository, not to the agent's mood: better steering files, faster checks,
missing skills, a stale index.

## Gather evidence

Review the session for moments where:

- the agent searched more than twice for the same thing, or found it by luck;
- the agent used a wrong name, a wrong command or an outdated assumption;
- a check failed late that could have failed early, or no check existed;
- the user corrected the agent on a convention that is not written down;
- a step was done by hand that a command, script, skill or shard could do.

Check `brok status` and `brok doctor` too: an index that is stale or missing
embeddings explains many slow searches.

## Classify each finding

| Area | Typical fix |
|---|---|
| Navigation | Add the concept to `GLOSSARY.md`, re-run `brok index`, name things consistently |
| Steering | Add or tighten a rule in `AGENTS.md` (one line, imperative) |
| Checks | Add a test, a lint, a CI step or a pre-commit hook |
| Skills | Adjust a workspace skill, or create one with `brok skill new` |
| Automation | A shard reacting to bus events (`brok shard new`) |
| Tooling | A script in `scripts/`, a better error message, a doc fix |

## Report

Most severe first, at most seven items:

```
1. [severity: high|medium|low] <area>: <what went wrong, with the moment it happened>
   Fix: <the concrete change, with the file and the exact text or command>
```

Then ask which ones to apply. Apply only the approved ones, each as a small,
separate change, and show the diff.
