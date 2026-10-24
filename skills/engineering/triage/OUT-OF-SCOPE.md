# The out-of-scope knowledge base

`.out-of-scope/` at the repository root records ideas the project decided not
to pursue, one concept per file, so the same request is not re-litigated
every few months.

## When to write an entry

Only for a **rejected enhancement**: the idea is understood and the answer is
no. Do not write one for bugs, for duplicates of open issues, or for requests
that are already implemented (point to the code instead).

## File format

`.out-of-scope/<concept-in-kebab-case>.md`:

```markdown
# <Concept, stated neutrally>

<One paragraph: what was asked for.>

## Why this is out of scope

<The reasoning, tied to SCOPE.md or an ADR. Say what users can do instead:
a shard, a workspace skill, configuration, a fork.>

## Prior requests

- #123: "<issue title>"
```

## Using it during triage

- Before recommending anything, read the titles of every file in
  `.out-of-scope/` and open the ones that resemble the request.
- If a match exists, recommend `wontfix`, link the file in the comment, and
  append the issue to its **Prior requests** list.
- If a rejected request is genuinely different from every entry, create a new
  file and link it.
- If the maintainer reverses a decision, delete the entry in the same change
  that implements the idea.
