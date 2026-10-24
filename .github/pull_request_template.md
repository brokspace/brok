## What and why

<!-- What does this change and which problem does it solve? Link the issue: "Closes #123". -->

## Evidence it works

<!-- Commands run, new tests, before/after output, platforms checked. -->

## Risk

<!--
- Door: two-way (easy to revert) or one-way (protocol, manifest, on-disk format, public API, config keys)?
- Blast radius: who or what breaks if this is wrong?
-->

## Checklist

- [ ] PR title follows Conventional Commits (`feat(index): ...`, `fix(bus): ...`)
- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] User-visible changes are in `CHANGELOG.md` (Unreleased) and the docs
- [ ] Protocol, manifest or config changes are backwards compatible, or an ADR / issue discusses the migration
- [ ] Added, renamed or removed skills are reflected in `skills/README.md` and the bucket `README.md`
- [ ] Both builds pass when relevant: default features and `--no-default-features`
