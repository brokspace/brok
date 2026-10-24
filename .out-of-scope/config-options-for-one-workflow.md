# Configuration options that serve one workflow

New `brok.yml` keys, CLI flags or environment variables that tune Brokspace to one person's or one team's way of working are declined.

## Why this is out of scope

"Zero complex configuration" is a product promise. Every option doubles a code path to test, appears in the reference docs, and is one more thing a new user has to understand before `brok init` feels finished. The project prefers a better default over a knob.

What to do instead:

- if the default is wrong for most people, open a bug or idea arguing for a **better default**;
- if it is your workflow, encode it where workflows belong: your own **skill** (`brok skill new`), your own **shard** (`brok shard new`), or the free-form `shards.<name>.settings` of a shard you control.

## Prior requests

None yet.
