---
name: write-shard
description: "Build a Brokspace shard: a small process on the local event bus that reacts to workspace events. Use when the user wants to automate something on file changes, add a custom agent or integration to Brokspace, edit a shard.json, or work with brok-sdk."
---

# Write a shard

A shard is a standalone program that `brok run` starts, connects to the local
event bus and restricts to the topics its `shard.json` declares. Most shards
are under 200 lines of Rust with `brok-sdk`.

## 1. Scaffold

```sh
brok shard new <name>            # Rust project: Cargo.toml, shard.json, src/main.rs
brok shard new <name> --skills-only   # only agent skills, no process
```

Names: lowercase, digits and dashes, starting with a letter.

## 2. Decide events and permissions

Write down, before coding:

- **Subscribes to**: which events trigger work (`file:changed` is the usual
  one). Use exact topics; wildcards only when needed.
- **Publishes to**: your own namespace, named after the shard
  (`license:violation`). `system:*` is reserved.
- **Least privilege**: only the paths it reads, `network: false` unless it
  calls a service, only the programs (`process`) and environment variables
  (`env`, e.g. `MY_TOOL_*`) it needs. Users see these at install time.

See [MANIFEST.md](MANIFEST.md) for an annotated `shard.json`.

## 3. Code the loop

```rust
use brok_sdk::prelude::*;

#[tokio::main]
async fn main() -> Result<(), ClientError> {
    let (client, mut inbox) = BrokClient::connect_from_env("my-shard", env!("CARGO_PKG_VERSION")).await?;
    client.subscribe([TopicPattern::new("file:changed")?]).await?;
    while let Some(event) = inbox.recv().await {
        if event.is::<Shutdown>() { break; }
        if let Ok(change) = event.decode::<FileChanged>() {
            // inspect change.changed (workspace-relative paths, `/` separators)
            // client.emit(&MyFinding { .. }).await?;
        }
    }
    Ok(())
}
```

- Define your payloads as serde structs implementing `brok_sdk::Payload`
  (`const TOPIC: &'static str = "my-shard:finding";`).
- Ask the index: `client.call::<SearchRequest, SearchResults>(&req, timeout)`
  (add `index:search` to `publishes_to`).
- Answer requests with `client.reply(&request_event, &payload)`; only the
  requester receives it.
- Log to stderr; `brok run` prefixes lines with the shard name.
- Exit within three seconds of `Shutdown`.

## 4. Test

- Unit-test the pure logic (parsing, matching) without the bus.
- Live: `brok shard install ./<name>`, then `brok run` in one terminal and
  `brok bus tail '<namespace>:*'` in another. Inject events with
  `brok bus emit file:changed '{"changed":["src/main.rs"],"deleted":[]}'`.
- Logs: `.brokspace/logs/<name>.log`.

## 5. Release

Tag a version, build binaries per platform (the template repo's release
workflow does this), and add each to `shard.json` `releases` with its SHA-256
so users install a verified binary instead of building from source. Publish
the repository; users install with `brok shard install <git-url> --tag vX.Y.Z`.
