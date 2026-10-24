# ADR format

File: `docs/adr/NNNN-short-title-in-kebab-case.md`, numbered sequentially.

```markdown
# NNNN. <Decision stated as a fact>

- Status: accepted | superseded by [NNNN](NNNN-other.md)
- Date: YYYY-MM-DD

## Context

What forced a decision: the problem, the constraints, the forces in tension.
Facts, not advocacy.

## Decision

What we do, in the present tense. "Shards depend on brok-sdk from crates.io;
local development patches it to the sibling checkout."

## Alternatives considered

- **Option**: why it lost, in one or two sentences.

## Consequences

What becomes easier, what becomes harder, and what we must now do or watch.
```

Rules:

- One decision per ADR. Short is fine: a good ADR is often under 30 lines.
- The title is the decision, not the question ("Use RRF to fuse rankings",
  not "Ranking fusion").
- Superseding: set the old one's status to `superseded by`, and say in the
  new one what it replaces. Never rewrite history.
