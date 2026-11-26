# Write your first shard

This tutorial builds a small shard from nothing: it reports source files without a license header and answers "which files mention this?" using brok's index. Every file below is complete and has been compiled and run. It takes about 20 minutes. The [shard reference](shards.md) has every rule and field.

## What a shard is

A shard is a program of your own that `brok` starts next to its other parts. It talks to them through a local **event bus**: it subscribes to events, does its work and publishes new events. It never links against brok, and it can only publish and receive the topics its manifest (`shard.json`) lists.

```text
brok-agent ──file:changed──▶ bus ──▶ license-header ──license:missing──▶ bus ──▶ anyone subscribed
                                          │
                                          └──index:search──▶ index service
```

Choose a shard when something should **react** to the workspace or offer a service to other parts. If you only want to teach your coding agent a workflow, write a [skill](skills.md) instead: no code to build.

You need brok 0.8 or newer, [Rust](https://rustup.rs) and git.

## 1. Create the project

The quickest start is a copy of the [template](https://github.com/brokspace/brok-shard-template), a complete project with tests, CI and release automation:

```sh
brok shard new license-header
```

This tutorial writes each file so you see what it is for. With the template, replace its files with these, or just read along.

```text
license-header/
  Cargo.toml       dependencies
  shard.json       what the shard may do
  src/check.rs     your logic, tested without brok
  src/payload.rs   the events you publish and receive
  src/main.rs      the wiring
```

`Cargo.toml`:

```toml
[package]
name = "license-header"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1"
brok-sdk = "0.3"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## 2. Declare what it may do

`shard.json` is the contract users approve when they install your shard. Ask for the least you need.

```json
{
  "name": "license-header",
  "version": "0.1.0",
  "description": "Reports source files without the license header",
  "license": "MIT",
  "brok": ">=0.8.0, <1.0.0",
  "entrypoint": "bin/license-header",
  "permissions": {
    "filesystem": { "read": ["."], "write": [] },
    "network": false
  },
  "subscribes_to": ["file:changed", "license:check"],
  "publishes_to": ["license:missing", "license:report", "index:search"],
  "build": { "command": ["cargo", "build", "--release"], "artifact": "target/release/license-header" }
}
```

- `brok` is the range of brok versions you support. If it excludes the user's version, the install is refused.
- `subscribes_to` and `publishes_to` are enforced by the bus. `index:search` is listed because the shard asks the index a question.
- `network` is `false`: this shard talks to nobody outside the machine.

## 3. Write the logic first

Keep the interesting part free of brok, so testing it needs nothing running. `src/check.rs`:

```rust
/// Whether the first lines of `text` carry `header`.
pub fn has_header(text: &str, header: &str) -> bool {
    text.lines().take(5).any(|line| line.contains(header))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_only_at_the_top_of_the_file() {
        assert!(has_header(
            "// SPDX-License-Identifier: MIT\nfn main() {}",
            "SPDX-License-Identifier"
        ));
        assert!(!has_header(
            "1\n2\n3\n4\n5\n// SPDX-License-Identifier: MIT",
            "SPDX-License-Identifier"
        ));
    }
}
```

Run `cargo test`.

## 4. Define your events

Each event is a serde type bound to a topic. Use your own prefix (`license:`) so you never clash with others. `src/payload.rs`:

```rust
use brok_sdk::Payload;
use serde::{Deserialize, Serialize};

/// A changed file without the header.
#[derive(Debug, Serialize, Deserialize)]
pub struct Missing {
    pub path: String,
}

impl Payload for Missing {
    const TOPIC: &'static str = "license:missing";
}

/// "Which files mention this?", answered with a `Report`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Check {
    pub query: String,
}

impl Payload for Check {
    const TOPIC: &'static str = "license:check";
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub files: Vec<String>,
}

impl Payload for Report {
    const TOPIC: &'static str = "license:report";
}
```

## 5. Wire it to the bus

`src/main.rs`:

```rust
mod check;
mod payload;

use std::path::Path;
use std::time::Duration;

use brok_sdk::prelude::*;

use crate::payload::{Check, Missing, Report};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `brok run` provides the address and token through the environment.
    let (client, mut inbox) =
        BrokClient::connect_from_env("license-header", env!("CARGO_PKG_VERSION")).await?;

    // `shards.license-header.settings` in brok.yml.
    let header = client
        .context()
        .settings
        .get("header")
        .and_then(|value| value.as_str())
        .unwrap_or("SPDX-License-Identifier")
        .to_owned();
    let root = client.context().workspace_root.clone();

    client
        .subscribe([
            TopicPattern::new("file:changed")?,
            TopicPattern::new("license:check")?,
        ])
        .await?;
    eprintln!("ready");

    while let Some(event) = inbox.recv().await {
        if event.is::<Shutdown>() {
            break;
        }
        if event.is::<FileChanged>() {
            on_change(&client, &root, &header, event.decode()?).await?;
        } else if event.is::<Check>() {
            let check: Check = event.decode()?;
            // Ask brok's index service (needs `index:search` in `publishes_to`).
            let search = SearchRequest::new(check.query).limit(5);
            let found: SearchResults = client.call(&search, Duration::from_secs(10)).await?;
            let files = found.hits.into_iter().map(|hit| hit.path).collect();
            // Published for anyone subscribed. To answer only the shard that asked,
            // use `client.reply(&event, &payload)` instead.
            client.emit(&Report { files }).await?;
        }
    }
    Ok(())
}

async fn on_change(
    client: &BrokClient,
    root: &Path,
    header: &str,
    change: FileChanged,
) -> anyhow::Result<()> {
    for path in change.changed.iter().filter(|path| path.ends_with(".rs")) {
        // A file may be gone or not be text: skip it, do not crash.
        let Ok(text) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        if !check::has_header(&text, header) {
            client.emit(&Missing { path: path.clone() }).await?;
        }
    }
    Ok(())
}
```

What happens, in order:

1. `connect_from_env` joins the bus with the credentials brok gives the process.
2. `client.context()` holds the workspace root and your settings from `brok.yml`.
3. `subscribe` names the topics you want; they must be in the manifest.
4. The loop handles one event at a time. Always exit on `Shutdown`.
5. `client.call` sends a request and waits for the typed answer; here the index search.

Log to **stderr** (`eprintln!` or `tracing`): brok captures it.

## 6. Run it

In a project you can experiment in:

```sh
brok init                                   # once per project
brok shard install ../license-header        # shows the permissions and asks first
brok start
```

Watch your events in a second terminal:

```sh
brok bus tail 'license:*'
```

Then give it something to do:

```sh
echo 'fn refund() {}' >> src/billing.rs       # brok-agent publishes file:changed
brok bus emit license:check '{"query":"invoice"}'
```

The output looks like this:

```text
license:missing from license-header {"path":"src/billing.rs"}
license:check from operator {"query":"invoice"}
license:report from license-header {"files":["src/billing.rs"]}
```

You can also publish `file:changed` yourself, which is handy when brok-agent is not installed:

```sh
brok bus emit file:changed '{"changed":["src/billing.rs"],"deleted":[]}'
```

## 7. Configure it

Settings live in the user's `brok.yml`, under your shard's name:

```yaml
shards:
  license-header:
    settings:
      header: "Copyright Acme"
```

Run `brok restart` after changing them. Never put secrets there, since `brok.yml` is often committed: name an environment variable in `permissions.env` and read it with `std::env::var`.

## 8. Change, reinstall, debug

```sh
brok shard install ../license-header --force   # --force: same version, new build
brok restart
```

The shard's log is `.brokspace/logs/license-header.log`. For verbose logs, `brok stop` and then `brok run -v` in the foreground.

| You see | Why and what to do |
|---|---|
| `requires brok ..., this is brok ...` | Your `brok` range excludes this version. Widen it. |
| Nothing arrives | Check `brok status`, the topic in `subscribes_to`, and publish the event with `brok bus emit`. |
| A publish or subscribe is rejected | The topic is missing from the manifest. Add it, reinstall, restart. |
| A variable is empty | The environment is scrubbed: list it in `permissions.env`. |
| A reply never shows in `bus tail` | `client.reply` goes only to the shard that asked. Publish with `emit` if others should see it. |
| Your change has no effect | Reinstall with `--force`, then `brok restart`. |

## 9. Share it

1. Push the project to a git repository. Users can already install it from source: `brok shard install <git-url>`.
2. Tag a version. The template's release workflow builds binaries for Linux, macOS and Windows with SHA-256 checksums.
3. Add them to `releases` in `shard.json`, so users get a verified binary and need no compiler.

## Learn from Brokspace's shards

| Shard | Read it to learn |
|---|---|
| [brok-shard-template](https://github.com/brokspace/brok-shard-template) | The smallest complete shard: react to changed files, publish findings, test, release. |
| [brok-agent](https://github.com/brokspace/brok-agent) | The system shard: answers requests with streamed replies (`ask.rs`), calls the index, uses the user's LLM settings, watches the file system and redacts secrets before anything leaves the machine. |
| [brok-obsidian](https://github.com/brokspace/brok-obsidian) | A network integration done carefully: a key from the environment, HTTPS with a pinned certificate, its own topics, a data folder, hostile input validation and tests against a fake server. |

## Before you publish: a checklist

- The manifest asks for nothing the shard does not use.
- Logic is tested without the bus, and a test checks that the topics in code are in the manifest (the template has one).
- Bad input never crashes the shard: a missing or binary file is skipped.
- Secrets come from the environment and are never logged.
- The README says what the shard reads, what it sends and where.
- Large work does not block the loop: use `tokio::task::spawn_blocking` for heavy file work.
