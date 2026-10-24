# Security policy

## Supported versions

Brokspace is pre-1.0: security fixes land on the latest minor release only.

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's
[private vulnerability reporting](https://github.com/brokspace/brok/security/advisories/new)
with:

- the affected component (`brok`, `brok-sdk`, `brok-agent`, a shard, …) and version;
- steps to reproduce or a proof of concept;
- the impact as you understand it.

We acknowledge reports within 3 working days and aim to ship a fix within
30 days, crediting you in the advisory unless you prefer otherwise.

## Scope

Particularly interesting:

- bypassing topic permissions, spoofing event sources or reading replies
  meant for another client on the bus;
- connecting to a bus without a valid token, or from another user account;
- secrets leaking into shard environments, logs or LLM prompts;
- path traversal or code execution through `shard.json`, `brok.yml` or
  `brok shard install` beyond what the user approved.

Known, documented limitation: shard file system and network permissions are
not yet enforced by the operating system (see `docs/shards.md#security`).
