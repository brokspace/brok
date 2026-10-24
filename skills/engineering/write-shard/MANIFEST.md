# Annotated shard.json

JSON has no comments, so the notes are below the example.

```json
{
  "$schema": "https://raw.githubusercontent.com/brokspace/brok-sdk/main/shard.schema.json",
  "name": "license-check",
  "version": "0.1.0",
  "description": "Flags source files without a license header",
  "license": "MIT",
  "repository": "https://github.com/acme/license-check",
  "brok": ">=0.1.0, <0.2.0",
  "entrypoint": "bin/license-check",
  "permissions": {
    "filesystem": { "read": ["src/"], "write": [] },
    "network": false,
    "process": [],
    "env": []
  },
  "subscribes_to": ["file:changed"],
  "publishes_to": ["license:violation"],
  "skills": ["skills"],
  "build": { "command": ["cargo", "build", "--release"], "artifact": "target/release/license-check" },
  "releases": {
    "linux-x86_64": { "url": "https://github.com/acme/license-check/releases/download/v0.1.0/license-check-linux-x86_64", "sha256": "<64 hex chars>" }
  }
}
```

| Field | Notes |
|---|---|
| `name` | Unique in the workspace, `^[a-z][a-z0-9-]{1,63}$`. |
| `version` | SemVer. `brok shard install` refuses downgrades without `--force`. |
| `brok` | Runtime versions you support. Keep it tight while brok is 0.x. |
| `entrypoint` | Path inside the installed shard; `.exe` is added on Windows. Omit for skill-only shards. |
| `permissions.filesystem` | Workspace-relative paths. No absolute paths, no `..`. |
| `permissions.network` | `false` unless you call a service. |
| `permissions.process` | Bare program names you run, e.g. `git`. |
| `permissions.env` | Variables passed through: exact names, `PREFIX_*`, `*_SUFFIX`. `BROK_*` is reserved. |
| `subscribes_to` | Topic patterns; the bus drops anything else. |
| `publishes_to` | Your namespace. Publishing to `*` or `system:*` is rejected. |
| `skills` | Directories whose subfolders each hold a `SKILL.md`. |
| `build` | Program plus arguments, run without a shell, in the repository. |
| `releases` | Keys are `<os>-<arch>` as Rust names them (`linux-x86_64`, `macos-aarch64`, `windows-x86_64`); `https` only; the SHA-256 is verified on install. |

A skill-only shard is just `name`, `version`, `description` and
`"skills": ["skills"]`.
