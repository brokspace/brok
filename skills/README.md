# Brokspace skills

A **skill** is a small, focused instruction set for a coding agent: a folder
with a `SKILL.md` file (the [Agent Skills](https://agentskills.io) format) and,
optionally, reference files next to it. Skills turn good engineering practice
into something an agent can repeat: align before building, slice work into
tracer bullets, test at agreed seams, review against the spec.

Brokspace ships a curated set of skills inside the `brok` binary and lets you
add your own, per workspace or as installable **skill shards**.

## Buckets

| Folder | Purpose | Embedded in `brok` |
|---|---|---|
| [`engineering/`](engineering/README.md) | Daily code work: specs, tickets, TDD, review, triage, shards | yes |
| [`productivity/`](productivity/README.md) | Non-code workflow: interviews, handoffs | yes |
| [`in-progress/`](in-progress/README.md) | Beta skills, public for feedback | no |

`engineering/` and `productivity/` are the **promoted** buckets. Every skill in
them is listed in its bucket `README.md` and in [`ask-brok`](engineering/ask-brok/SKILL.md),
the router skill.

## User-invoked and model-invoked

Every skill sits on one side of a single line: who can start it.

- **User-invoked** skills run only when a person types them (`/to-spec`,
  `/triage`). They orchestrate a flow. Their frontmatter carries
  `disable-model-invocation: true`, and `agents/openai.yaml` carries
  `policy.allow_implicit_invocation: false`.
- **Model-invoked** skills can be started by a person or picked up by the agent
  on its own when the task fits (`tdd`, `brok-context`). They hold reusable
  discipline, so their descriptions list the situations that should trigger
  them.

A user-invoked skill may call model-invoked skills. It never calls another
user-invoked skill: it tells the person to run it instead.

## Using the skills

```sh
brok skill list                      # built-in, workspace and shard skills
brok skill show tdd                  # print a skill
brok skill run code-review "review the diff against main"   # run on your local LLM
brok skill export --agent claude     # copy into .claude/skills/
brok skill export --agent agents     # copy into .agents/skills/ (Codex, Cursor and others)
brok skill export --agent cursor     # copy into .cursor/skills/
```

`brok skill run` sends the skill to `brok-agent`, which answers with your
configured model (Ollama, LM Studio or a hosted provider) and retrieves
workspace context from the Brokspace index first. It needs `brok run`.

Inside an exported agent, start a user-invoked skill with its slash name
(`/setup-brokspace`) and let the agent pick model-invoked ones by itself.

First time in a repository? Run `/setup-brokspace` once. It records which issue
tracker you use, your triage label names and where docs go, so the other
skills know where to publish.

## Adding and overriding skills

- **Workspace skills**: `brok skill new <name>` creates
  `.brokspace/skills/<name>/SKILL.md`. A workspace skill with the same name as a
  built-in one overrides it, which is how you adapt a built-in to your team.
- **Skill shards**: a shard whose `shard.json` lists `"skills": ["skills"]` and
  no `entrypoint` only contributes skills. Create one with
  `brok shard new <name> --skills-only`, share it as a git repository, install
  it with `brok shard install <git-url>`.
- **Contributing a built-in**: read the [`write-skill`](engineering/write-skill/SKILL.md)
  skill and the project `SCOPE.md` first. Built-ins are deliberately few.

## Credits

Several workflows here are adapted from
[mattpocock/skills](https://github.com/mattpocock/skills) (MIT). See
[CREDITS.md](CREDITS.md).
