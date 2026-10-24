---
name: tdd
description: "Test-driven development with a red-green loop, one vertical slice at a time. Use when building a feature or fixing a bug test-first, when the user mentions TDD, red-green, or regression tests, or when another skill asks for tests at agreed seams."
---

# Test-driven development

Red, then green, one small slice at a time, always through a public
interface. The tests you keep should read like a specification of what the
code does, and survive any refactor that keeps that behavior.

Read `GLOSSARY.md` if present: test names use the project's terms.

## Seams first

A seam is the public boundary a test exercises: a function's signature, a CLI
command, an HTTP route, a bus topic, a file format. Before the first test:

1. List candidate seams, each with one line on what a test there catches and
   what it misses.
2. Get the user's confirmation (unless another skill already agreed them).
3. Never test below an agreed seam: no private functions, no internal
   collaborators.

## The loop

1. **Red**: write one test for the next behavior. Run it. It must fail, and
   for the reason you expect. A test that passes immediately proves nothing;
   rethink it.
2. **Green**: write the least code that makes it pass. No extra branches, no
   future-proofing.
3. **Run everything** that is fast. Keep the suite green before the next test.
4. Next behavior. Each test is a tracer bullet: let what you learned shape the
   next one.

Refactoring is not part of this loop. It happens afterwards, during review.

## Good tests

- Assert observable outcomes (returned values, output, files written, events
  published), not which helpers were called.
- Expected values come from an independent source: a literal, a worked
  example, the spec. Never recompute them the way the code does.
- One reason to fail per test. Name it after the behavior:
  `rejects_tokens_signed_with_unknown_keys`.
- Use real collaborators that are fast and local (temp dirs, in-memory
  stores, a local socket). Fake only what is slow, flaky or external, and fake
  it at the seam.

## Anti-patterns

- **Writing all the tests first**, then all the code: tests end up checking
  imagined shapes instead of behavior.
- **Mocking internals**: the test breaks on refactors that keep behavior.
- **Tautologies**: the assertion repeats the implementation, so it can never
  fail.
- **Snapshotting everything**: big snapshots get approved without reading.

See [tests.md](tests.md) for good and bad examples in Rust and TypeScript.
