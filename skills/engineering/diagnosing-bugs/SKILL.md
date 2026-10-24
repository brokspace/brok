---
name: diagnosing-bugs
description: "Disciplined diagnosis of hard bugs and performance regressions. Use when a bug is not obvious after a first look, when a fix attempt already failed, when behavior is intermittent or environment-specific, or when something got slower."
---

# Diagnosing bugs

Do not guess fixes. Build a loop that shows the bug, shrink it, then change
one thing at a time. Each phase has an exit condition; do not skip ahead.

## 1. Make it red

Find a command that fails because of the bug, every time, in seconds:
a failing test, a script, a `curl`, a benchmark with a threshold.

- Use the reporter's exact steps and inputs first.
- Intermittent? Loop it (`for i in $(seq 50); do ...; done`) and record the
  failure rate.
- Performance? Measure before touching anything and write the number down.

Exit: you can run one command and watch it fail. If you cannot reproduce it,
stop and report what you tried and what extra information would help.

## 2. Minimise

Remove inputs, config, steps and code paths until removing anything else
makes the failure disappear. Note what turned out not to matter.

## 3. Locate

- Search for the code paths involved: `brok search "<behavior in question>"`
  (or the `brok_search` tool), then read the hits around the reported symptom.
- Check what changed: `git log -p --since=... -- <paths>`, or bisect with the
  red command (`git bisect run <command>`).

## 4. Hypothesise and instrument

Write down two or three hypotheses, each with a prediction you can check.
Test the cheapest first by adding logs, assertions or a debugger breakpoint,
not by editing behavior. Discard hypotheses the evidence rejects, out loud.

Exit: one hypothesis explains every observation, including the ones from
step 2.

## 5. Fix and prove

1. Turn the red command into a regression test at a public seam (call the
   Skill tool with "tdd" for what a good one looks like).
2. Make the smallest fix that turns it green.
3. Prove the test guards the bug: revert the fix, see red, restore it.
4. Run the full suite. Remove temporary instrumentation.

## Report

Root cause in one or two sentences, the evidence for it, the fix (paths), the
regression test, and anything suspicious you noticed but did not fix.
