# Domain-specific features built into the core

Requests to add domain-specific behaviour directly to `brok` or `brok-agent` are declined when that behaviour can be built as a shard or a skill. Examples: a security or license scanner, a Jira/Linear/Slack bot, a team-specific review policy, a framework-specific code generator, a language-specific linter bridge.

## Why this is out of scope

The core is the runtime every workspace shares: the index, the bus, the shard lifecycle, skills and integrations. Each feature added there is maintained forever, shipped to every user and weighs on the "simple, zero-config" promise.

Shards exist precisely so this behaviour can live next to the core instead of inside it: a shard subscribes to `file:changed`, queries the index with `index:search`, publishes its own events and is installed with one command. Skills do the same for agent workflows. Building it as a shard also lets its author release on their own schedule.

If a shard **cannot** be built because the SDK or the runtime lacks something, that gap is in scope: open an issue on [brok-sdk](https://github.com/brokspace/brok-sdk/issues) describing the shard you are building and where it got stuck.

## Prior requests

None yet.
