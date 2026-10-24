---
name: brok-context
description: "Explore a codebase through the Brokspace index. Use when you need to find where something is implemented, understand a concept across files, locate callers or similar code, or gather context before changing code, in any repo with a .brokspace/ directory or the brok_search tool."
---

# Brokspace context

Brokspace keeps a local index of the workspace (every git submodule
included): code cut along its syntax tree, one function, method, class or
section per chunk, searchable by meaning and by words. Use it before reading
files blindly.

## Which tool

| Need | Use |
|---|---|
| A concept or behavior ("where are tokens validated") | `brok search "<question>"` or the `brok_search` tool |
| An identifier you half remember | `brok search "parse config"` (matches `parseConfig`, `parse_config`) |
| An exact string, regex or every occurrence | `grep` / `rg`, not the index |
| A summarized answer with sources | `brok ask "<question>"` (needs `brok run`) |
| Is the index current? | `brok status` |

## Searching well

1. Phrase the query as the code would describe itself: the behavior plus
   domain terms from `GLOSSARY.md`.
2. Start with the default limit. Add `-n 15` only when results look relevant
   but incomplete.
3. Read each hit's header: `score path:start-end kind symbol`. A `method` hit
   with a `parent` tells you the class or impl it belongs to.
4. Need the whole chunk? Re-run with `--full`, or open the file at those
   lines. Need machine-readable output? `--json`.
5. Two queries with different wording beat one long query. Try the user's
   words, then the code's words.
6. When hits come from a submodule partition, keep that in mind for build and
   test commands, which may run inside the submodule.

## Freshness

`brok run` re-indexes changed files within seconds. Without it, results may
lag edits made in this session: run `brok index` (incremental, usually fast)
before trusting a search for code you just changed. If `brok status` shows
"bm25" instead of "hybrid", semantic matching is off; prefer exact identifier
words in queries.

## Reporting

Cite every claim about the code as `path:line` (or `path:start-end`). Say
which queries you ran when you conclude something does not exist, so the
user can judge how hard you looked.

If neither `brok` nor `brok_search` is available, say so once and fall back to
`grep` and directory listings.
