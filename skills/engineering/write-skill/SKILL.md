---
name: write-skill
description: "Author or improve an agent skill (a SKILL.md) for Brokspace and coding agents. Use when the user wants to capture a repeatable workflow as a skill, edit a SKILL.md, decide whether something should be a skill, or publish skills as a skill shard."
---

# Write a skill

A skill is a short instruction file an agent loads only when it is relevant.
Good skills are small, explicit and composable. Before writing one, check
whether the workflow already composes from existing skills (`brok skill
list`); if it does, a line in `AGENTS.md` may be enough.

## 1. Create it

```sh
brok skill new <name>                  # .brokspace/skills/<name>/SKILL.md (this workspace)
brok skill new <name> --user-invoked   # only a person can start it
```

A workspace skill with the same name as a built-in overrides it.

## 2. Choose who can invoke it

- **User-invoked**: it orchestrates a flow the person should decide to start
  (publishing to a tracker, a multi-step interview). Frontmatter
  `disable-model-invocation: true`; in `agents/openai.yaml` set
  `policy.allow_implicit_invocation: false`. Description: one human-facing
  line.
- **Model-invoked** (default): a discipline the agent should apply on its own
  when the situation fits. Description: what it does plus the triggers
  ("Use when ..., when the user mentions ..., or when ...").

Test for model-invoked: would it help if the agent reached for it without
being asked? If not, make it user-invoked.

## 3. Write the body

- Frontmatter: `name` equals the folder name (kebab-case); `description` in
  quotes if it contains a colon.
- Imperative steps, numbered when order matters. Each step has an exit
  condition or an output.
- Concrete commands and file paths, not advice. A small local model
  (7B parameters) must be able to follow it: no implied steps, no long
  prose.
- Keep `SKILL.md` under about 100 lines. Move examples and formats into
  reference files in the same folder and link them.
- Depend on other skills by name with "Call the Skill tool with
  \"<name>\"", and only on model-invoked ones. For a user-invoked
  prerequisite, tell the user to run it.
- Say what to output at the end.

## 4. Test it

- `brok skill show <name>` to check it parses.
- `brok skill run <name> "<realistic input>"` on the local model, then in
  your coding agent after `brok skill export --agent <agent>`.
- Fix the instruction that caused each mistake; do not add caveats for every
  case.

## 5. Share it

Move it into a skill shard (`brok shard new <name> --skills-only`, put the
skill folder under `skills/`), push the repository, and teammates install it
with `brok shard install <git-url>`.
