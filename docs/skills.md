# Skills

A **skill** is a small, reusable workflow written for AI agents: a folder
with a `SKILL.md` (YAML frontmatter `name` + `description`, then
instructions) and optional reference files. Brokspace uses the same
[Agent Skills](https://github.com/mattpocock/skills) format as Claude Code,
Codex, Cursor and GitHub Copilot, so a skill written once works everywhere.

```text
code-review/
├── SKILL.md            name, description, instructions
├── agents/openai.yaml  display metadata for Codex (optional)
└── CHECKLIST.md        reference material linked from SKILL.md (optional)
```

## Two kinds

| | User-invoked | Model-invoked |
|---|---|---|
| Who starts it | Only you, by typing `/name` | You, or the agent when the task fits |
| Frontmatter | `disable-model-invocation: true` | (default) |
| Role | Orchestrates a flow (`/to-spec`, `/triage`) | A reusable discipline (`tdd`, `code-review`) |
| Description | One human-facing line | Rich "Use when..." triggers |

## Built-in skills

`brok` ships a curated set, embedded in the binary (source:
[skills/](../skills/README.md)). They cover the whole development loop that
Brokspace itself uses: align, specify, slice, build test-first, review, ship.

```sh
brok skill list          # everything available, with its source
brok skill show tdd      # read one
```

Not sure where to start? Type `/ask-brok` in your coding agent and describe
your situation.

## Using skills

**In your coding agent** (the main way):

```sh
brok setup agents        # skills + MCP search tool, for the agents it detects
brok skill export        # skills only; --agent claude|agents|cursor|vscode|codex, --global
```

Skills land in `.claude/skills/` and the portable `.agents/skills/` (read by
Codex, Cursor, Copilot and others). Folders are marked `.brokspace-managed`
and refreshed on every export; brok never overwrites a folder it did not
create. Commit them to share with your team.

**On your local LLM**, through `brok-agent` (needs `brok run`):

```sh
brok skill run code-review "the changes in src/api since main"
brok skill run grill-me "plan: move sessions to redis"
```

The skill's instructions (and the reference files it links) become the
system prompt; Brokspace adds relevant code from the index as context.

## Writing your own

```sh
brok skill new release-notes                  # model-invoked, in .brokspace/skills/
brok skill new deploy --user-invoked          # a /deploy workflow only you trigger
```

Workspace skills in `.brokspace/skills/<name>/` **override** built-ins with
the same name, so you can tailor any built-in to your team. Keep skills
short and explicit: small local models follow them too. The built-in
`write-skill` skill walks your agent through it.

## Sharing skills: skill shards

A **skill shard** is a shard with no process, just skills. Publish it as a
git repository and anyone can install it:

```sh
brok shard new acme-skills --skills-only      # shard.json + skills/example/
# edit, push to GitHub, then on any machine:
brok shard install https://github.com/acme/acme-skills
brok skill list                               # source: shard:acme-skills
```

`shard.json` of a skill shard:

```json
{ "name": "acme-skills", "version": "0.1.0", "skills": ["skills"] }
```

A shard can also ship skills next to its process (`entrypoint` + `skills`).

## Precedence

When names collide, the first wins: workspace (`.brokspace/skills/`) >
installed shards > built-in. `brok skill list` reports shadowed skills.

## Credits

The built-in workflows are adapted from
[mattpocock/skills](https://github.com/mattpocock/skills) (MIT); see
[skills/CREDITS.md](../skills/CREDITS.md).
