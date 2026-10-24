# Shards

Shards are small programs that react to workspace events. Each one is a
separate process connected to the `brok run` event bus, restricted to the
topics its manifest declares.

The fastest way to write one is the
[brok-shard-template](https://github.com/brokspace/brok-shard-template)
repository; this page is the reference.

## Lifecycle

```sh
brok shard new my-shard                  # project from brok-shard-template
brok shard new my-skills --skills-only   # a skill shard: shard.json + skills/
brok shard install ./my-shard            # local directory (development)
brok shard install https://github.com/acme/brok-license-check --tag v1.0.0
brok run                                 # starts every enabled shard
brok bus tail 'license:*'                # watch what it publishes
brok shard remove brok-license-check
```

Installing:

1. clones the repository (`--depth 1`, optional `--tag`) or uses the local directory;
2. validates `shard.json` and the `brok` version requirement;
3. **shows the requested capabilities and asks for consent** (`--yes` to skip in scripts);
4. obtains the executable, in order of preference:
   - a release asset for this platform, verified against its SHA-256;
   - the manifest's `build.command`, run in the repository;
   - an `entrypoint` already present in the repository;
5. moves it into `.brokspace/shards/<name>/` and records the source, commit and
   binary hash in `shards.lock`.

Disable a shard without removing it in `brok.yml`:

```yaml
shards:
  brok-license-check:
    enabled: false
```

## `shard.json`

```json
{
  "$schema": "https://raw.githubusercontent.com/brokspace/brok-sdk/main/shard.schema.json",
  "name": "brok-security-auditor",
  "version": "1.0.0",
  "description": "Flags risky changes in source files",
  "license": "MIT",
  "repository": "https://github.com/acme/brok-security-auditor",
  "brok": ">=0.1.0, <0.2.0",
  "entrypoint": "bin/security-auditor",
  "args": [],
  "runtime": "native",
  "permissions": {
    "filesystem": { "read": ["src/"], "write": [] },
    "network": false,
    "process": ["git"],
    "env": ["SEMGREP_*"]
  },
  "subscribes_to": ["file:changed"],
  "publishes_to": ["security:alert"],
  "build": { "command": ["cargo", "build", "--release"], "artifact": "target/release/security-auditor" },
  "releases": {
    "linux-x86_64":   { "url": "https://github.com/acme/…/security-auditor-linux-x86_64", "sha256": "…" },
    "macos-aarch64":  { "url": "https://github.com/acme/…/security-auditor-macos-aarch64", "sha256": "…" },
    "windows-x86_64": { "url": "https://github.com/acme/…/security-auditor-windows-x86_64.exe", "sha256": "…" }
  }
}
```

| Field | Required | Rules |
|---|---|---|
| `name` | ✓ | `^[a-z][a-z0-9-]{1,63}$`, unique in the workspace |
| `version` | ✓ | SemVer |
| `entrypoint` | ✓¹ | Relative path inside the shard directory; `.exe` is appended on Windows |
| `skills` | ✓¹ | Directories whose subfolders each hold a `SKILL.md` ([skills](skills.md)) |
| `brok` | | SemVer requirement on the runtime version |
| `runtime` | | `native` (default) |
| `permissions.filesystem.read/write` | | Workspace relative paths, no `..`, no absolute paths |
| `permissions.network` | | `false` by default |
| `permissions.process` | | Bare program names the shard intends to run |
| `permissions.env` | | Variables to pass through: exact names, `PREFIX_*` or `*_SUFFIX`. `BROK_*` is reserved |
| `subscribes_to` / `publishes_to` | | Topic patterns; publishing to `system:*` (or `*`) is not allowed |
| `build.command` | | Program + arguments, executed **without a shell** |
| `releases.<os>-<arch>` | | `https://` URL and 64 hex SHA-256; keys use Rust's `std::env::consts` names (`linux-x86_64`, `macos-aarch64`, `windows-x86_64`, …) |

¹ A shard needs an `entrypoint`, `skills`, or both. Skill-only shards run no
process, so they cannot declare topics, `build` or `releases`.

Unknown fields are rejected, and all violations are reported at once.

## Topics

`namespace:name`, lowercase letters, digits, `-`, `_` and `.` for sub-topics:
`file:changed`, `agent:ask.chunk`. Patterns are exact topics or a prefix
ending in `*` (`agent:ask.*`, `file:*`, `*`). Pick a namespace named after
your shard for your own events. The well-known topics and their payloads are
listed in [architecture.md](architecture.md#well-known-topics).

## Runtime contract

`brok run` starts the entrypoint with:

- working directory = workspace root;
- an environment containing only OS basics (`PATH`, `HOME`, `TEMP`, …),
  `RUST_LOG`, the variables allowed by `permissions.env`, and:

| Variable | Value |
|---|---|
| `BROK_ENDPOINT` | Socket path / pipe name of the bus |
| `BROK_TOKEN` | The shard's session token (secret) |
| `BROK_SHARD_NAME` | Manifest name |
| `BROK_WORKSPACE` | Absolute workspace root |
| `BROK_FS_READ` / `BROK_FS_WRITE` | Declared paths, `;`-separated |
| `BROK_NETWORK` | `1` or `0` |

- stdout and stderr are captured line by line, prefixed with the shard name in
  the `brok run` output and appended to `.brokspace/logs/<name>.log`. Log to
  stderr.
- On `system:shutdown` (delivered without subscribing), exit within 3 seconds.
- A non-zero exit is a crash: the shard is restarted with backoff.

## Using the SDK

```rust
use brok_sdk::prelude::*;

#[tokio::main]
async fn main() -> Result<(), ClientError> {
    let (client, mut inbox) = BrokClient::connect_from_env("my-shard", env!("CARGO_PKG_VERSION")).await?;
    client.subscribe([TopicPattern::new("file:changed")?]).await?;

    while let Some(event) = inbox.recv().await {
        if event.is::<Shutdown>() {
            break;
        }
        if let Ok(change) = event.decode::<FileChanged>() {
            for path in change.changed {
                // … inspect the file, then publish your own event:
                // client.emit(&MyFinding { path, … }).await?;
            }
        }
    }
    Ok(())
}
```

- Typed payloads implement `brok_sdk::Payload` (a serde type plus its topic).
- `client.request(event)` returns a stream of replies; `client.call::<Req,
  Res>(&req, timeout)` waits for the first typed reply. Example: query the
  index with `client.call::<SearchRequest, SearchResults>(…)` (add
  `index:search` to `publishes_to`).
- `client.reply(&request, &payload)` answers a request; only the requester
  receives it.
- `client.context()` returns the workspace root, your `shards.<name>.settings`
  from `brok.yml` and the `llm` routing table.

Shards do not have to be written in Rust: anything that can open a Unix
socket / named pipe and exchange length-prefixed JSON frames works. The frame
format is documented in `brok-sdk/src/frame.rs` and `codec.rs`.

## Security

What the runtime **enforces**: topic ACLs, `source` stamping, reply routing,
the scrubbed environment, the working directory.

What is **declared but not yet enforced by the OS**: file system and network
access. Consent at install time is the safeguard until OS sandboxes (Landlock,
Seatbelt, AppContainer) land. `build.command` runs arbitrary code from the
shard repository (e.g. `build.rs`) — the consent prompt shows it before it
runs. Prefer shards that publish checksum-pinned `releases`.
