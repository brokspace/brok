#!/usr/bin/env bash
# Copies the built-in skills into the brok-skills crate so `cargo publish`
# packages them (crates cannot include files outside their directory).
# Run right before publishing; the copy is git-ignored.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rm -rf "$root/crates/brok-skills/skills"
cp -r "$root/skills" "$root/crates/brok-skills/skills"
echo "vendored $(find "$root/crates/brok-skills/skills" -name SKILL.md | wc -l) skills into crates/brok-skills/skills"
