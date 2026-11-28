# Shards in Python, TypeScript and Java

A shard is a program that talks to the local event bus, so it can be written in any language. Rust has
an SDK ([`brok-sdk`](https://github.com/brokspace/brok-sdk)). Other languages do not have one yet:
they speak the protocol below, which takes about forty lines.

Three complete shards, with no third party dependencies, are in
[`examples/shards`](https://github.com/brokspace/brok/tree/staging/examples/shards):

| Language | Needs | Files |
|---|---|---|
| Python | `python3` 3.8 or newer | one script |
| TypeScript | Node 22.13 or newer | `main.ts`, `build.mjs` |
| Java | JDK 17 or newer | `Main.java`, `Json.java`, `build.sh` |

Each one reports the `TODO` and `FIXME` lines of changed files and answers a question by searching the
index.

> Tested on Linux and macOS. **Windows is not supported by these examples**: they rely on a script
> entrypoint and a Unix socket, and a Windows shard needs a `.exe` entrypoint and a named pipe.

## Try one

From a workspace where `brok init` has run, with a copy of the brok repository next to it:

```sh
brok shard install ../brok/examples/shards/python    # or typescript, java
brok restart
brok bus tail 'todo-python:*'                        # leave it running
```

In another terminal:

```sh
brok bus emit todo-python:ask '{"query":"how are tokens validated"}'
echo '# TODO: check the error path' >> notes.py
brok bus emit file:changed '{"changed":["notes.py"]}'
```

With `brok-agent` installed, saving a file sends `file:changed` by itself. The shard's output is in
`.brokspace/logs/<name>.log`.

## The protocol

`brok run` starts your program in the workspace root with these variables:

| Variable | Value |
|---|---|
| `BROK_ENDPOINT` | Path of the Unix socket (a named pipe name on Windows) |
| `BROK_TOKEN` | Your session token. Do not log it |
| `BROK_SHARD_NAME` | The name in `shard.json` |
| `BROK_WORKSPACE` | Absolute workspace root |

Every message, in both directions, is a **frame**: a 4 byte big endian length, then that many bytes of
UTF-8 JSON. A frame may arrive in pieces, and one read may hold several. The limit is 16 MiB.

1. Send `hello` first:

   ```json
   {"type":"hello","protocol":1,"name":"todo-python","version":"0.1.0","token":"..."}
   ```

2. Receive `welcome`. Its `context` holds `workspace_root`, your `settings` from
   `shards.<name>.settings` in `brok.yml`, and the `llm` providers. Anything else here means the
   connection was refused.

3. Subscribe to topics listed in `subscribes_to`:

   ```json
   {"type":"subscribe","patterns":["file:changed","todo-python:ask"]}
   ```

4. Publish events on topics listed in `publishes_to`. You choose the `id` (a new UUID) and the time:

   ```json
   {"type":"publish","event":{"id":"<uuid>","topic":"todo-python:todo","timestamp_ms":1764200000000,"payload":{"path":"a.py","line":3,"text":"# TODO"}}}
   ```

5. Handle what the bus sends:

   | Frame | Meaning |
   |---|---|
   | `{"type":"event","event":{...}}` | A subscribed event, or a reply to one of yours |
   | `{"type":"error","code":"forbidden","message":"..."}` | The bus rejected a request, usually a topic missing from `shard.json` |

   A **reply** carries `correlation_id` equal to the `id` of your event, and reaches you without a
   subscription. That is how a search works: publish `index:search` with
   `{"query":"...","limit":5}`, and read `payload.hits` of the reply. To answer someone else's event,
   publish with its `id` as your `correlation_id`.

6. If you answer `agent:ask`: an event with topic `system:request.cancelled` and payload
   `{"request": "<event id>"}` means the client asking has left. Stop working on that request,
   including any model call. It arrives without a subscription.

7. When an event with topic `system:shutdown` arrives, or the connection closes, exit with code 0
   within 3 seconds. Any other exit code counts as a crash, and brok restarts the shard.

The payloads of the well-known topics are listed in
[architecture.md](architecture.md#well-known-topics). The frame types are defined in
[`brok-sdk/src/frame.rs`](https://github.com/brokspace/brok-sdk/blob/main/src/frame.rs).

## Packaging: brok installs one file

Besides `shard.json` and skills, an installed shard is the **single file** named by `entrypoint`.
Other files of your project are not copied, so put everything in it:

| Language | How the examples do it | With dependencies |
|---|---|---|
| Python | The script itself, starting with `#!/usr/bin/env python3` | Build a [`zipapp`](https://docs.python.org/3/library/zipapp.html) with a shebang |
| TypeScript | `build.mjs` removes the types and adds a shebang | Bundle with a tool such as esbuild into one file |
| Java | A shell line followed by the jar | Build a "fat" jar with Maven or Gradle first |

`build.command` and `build.artifact` in `shard.json` tell `brok shard install` how to produce that
file. The command runs on the user's machine and is shown before consent, so keep it short and without
downloads when you can. To skip building, publish the file in `releases` with its checksum.

Two details worth copying from the examples:

- **Paths come from events.** Resolve them and refuse anything outside the workspace.
- **Node module type.** The installed file lives inside the user's workspace, whose `package.json`
  decides whether Node reads it as CommonJS or as a module. Use `process.getBuiltinModule` (as the
  example does) or bundle to a form that works in both.

The environment is scrubbed: besides the basics such as `PATH` and `HOME`, a shard sees only the
variables it lists in `permissions.env` (the Java example asks for `JAVA_HOME`).

## Test your shard

`python3 examples/shards/test.py <path to brok> [python typescript java]` installs the examples in a
temporary workspace and checks what they publish. CI runs it on Linux and macOS; copy it as a starting
point for your own.
