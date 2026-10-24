# GLOSSARY.md format

```markdown
# <Project name>

One paragraph: what the project is and who it is for.

## Language

**Shard**:
A standalone process that joins the Brokspace event bus under the permissions
of its `shard.json` manifest.
_Avoid_: plugin, extension, addon

**Hot partition**:
The index database of one git submodule, rewritten when that submodule changes.
_Avoid_: shard index, sub-index

## Relationships

- A **Workspace** has one **Cold partition** and zero or more **Hot partitions**
- A **Shard** may provide **Skills**

## Flagged ambiguities

- "index" meant both the database and the act of indexing. Resolved: the
  database is the **Index**; the act is **indexing**.
```

Rules:

- Terms in bold, alphabetical within `## Language`.
- Definitions say what the thing *is*, in one or two sentences, without
  implementation details that change often.
- `_Avoid_` lists words that must not be used for the term.
- Every resolved ambiguity stays under `## Flagged ambiguities` so the
  history of the decision is visible.
