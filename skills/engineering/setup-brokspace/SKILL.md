---
name: setup-brokspace
description: Configure this repository for the Brokspace skills (issue tracker, triage labels, docs layout). Run once per repo.
disable-model-invocation: true
---

# Setup Brokspace

Prepare the current repository so the other skills know where to publish
issues, which label strings to apply and where domain docs live. Ask the
questions below one at a time, offer the recommended answer, write the files,
then summarize.

## 1. Check what exists

- `.brokspace/`: if missing, recommend `brok init` (indexing makes every other
  skill sharper) but continue either way.
- `docs/agents/issue-tracker.md`, `docs/agents/triage-labels.md`,
  `GLOSSARY.md`, `docs/adr/`: if present, show them and ask whether to keep or
  revise. Never overwrite without a yes.

## 2. Issue tracker

Ask which tracker holds this repo's issues. Detect a default: a GitHub remote
in `git remote -v` suggests GitHub, a GitLab remote suggests GitLab.

- **GitHub**: through the `gh` CLI. Ask whether external pull requests count
  as triage work, and who counts as external (non-collaborators by default).
- **GitLab**: through the `glab` CLI.
- **Local files**: one Markdown file per issue in `.scratch/issues/`, named
  `NNNN-slug.md`, with a `labels:` line at the top.
- **Other**: write down exactly how to create, read, label and close an issue.

Write `docs/agents/issue-tracker.md` with: the tracker, the CLI or folder,
copy-pasteable commands for create, view, comment, label and close, and the
external-PR rule.

## 3. Triage labels

The skills speak in canonical roles. Ask which label string the tracker uses
for each one; default to the canonical name.

Write `docs/agents/triage-labels.md` as a table with columns
`Role | Label in this repo | Meaning` covering: `bug`, `enhancement`,
`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`.
Offer to create missing labels (`gh label create <name>`).

## 4. Domain docs

Ask where the glossary and ADRs live. Defaults: `GLOSSARY.md` at the root and
`docs/adr/NNNN-title.md`. If `GLOSSARY.md` is missing, seed it with a title,
one paragraph on what the project is, and empty `## Language` and
`## Relationships` sections. Create `docs/adr/` if missing.

## 5. Point agents at it

Add (or extend) an `## Agent skills` section in `AGENTS.md` (create the file
if needed; `CLAUDE.md` may simply say "See AGENTS.md") that lists the three
docs above in one line each.

## 6. Finish

Summarize the files written. Recommend, in this order, only what is missing:
`brok init`, `brok index`, `brok setup agents` (exports skills and wires the
`brok_search` MCP tool into the user's coding agents), then `/ask-brok` to
pick a first flow.
