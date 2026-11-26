# Obsidian: memory and second brain

Brokspace memories are Markdown notes with a YAML front matter, and an
[Obsidian](https://obsidian.md) vault is a folder of Markdown notes. So an
Obsidian vault can be one more place Brokspace remembers things, and the
notes brok and your coding agents write show up in Obsidian next to the
rest of your thinking: graph, backlinks, search, Dataview, mobile.

Use it as an **optional** memory: nothing changes until you point brok at a
vault, and the default `private`, `shared` and `global` scopes keep working.

```text
  you / coding agent ──▶ brok memory add --scope obsidian ──▶ <vault>/Brokspace/*.md
                                                                      │
  brok search / brok ask / MCP  ◀── searched with your code ◀─────────┘
                                                                      │
  Obsidian (desktop, mobile, Sync)  ◀── same files ───────────────────┘
```

## Set it up (local vault)

A vault is any folder you opened in Obsidian (it contains a `.obsidian/`
folder). Tell brok where it is:

```sh
brok config set memory.obsidian.vault ~/Documents/Obsidian/Main
```

or in `brok.yml`:

```yaml
memory:
  obsidian:
    vault: ~/Documents/Obsidian/Main   # absolute, ~/... or relative to the workspace
    folder: Brokspace                   # where brok reads and writes, inside the vault
```

Then save to it with the `obsidian` scope:

```sh
brok memory add "Orders are idempotent per cart id" --kind decision --tag checkout \
  --scope obsidian --body "createOrder returns the existing order for a cart."
brok memory list --scope obsidian
brok memory search "idempotent"        # searches the vault folder with your code
```

Open Obsidian: the note is in `Brokspace/`, with its `kind`, `tags`,
`created` and `updated` as Obsidian properties.

What brok does and does not do with the vault:

| | |
|---|---|
| Writes | Only `<vault>/<folder>/` (default `Brokspace/`). |
| Reads | That folder; the whole vault only with `memory.obsidian.search: vault` (see [searching the whole vault](#searching-the-whole-vault)). Notes you add or edit by hand are searchable too. |
| Never touches | The rest of your vault (read only at most), and `.obsidian/` (your settings and plugins, never read). |
| Never creates | The vault itself. If `vault` does not exist, brok stops with an error instead of creating a lookalike folder (see [cloud vaults](#cloud-vaults)). |
| Forgetting | `brok memory forget` moves the note to `Brokspace/.forgotten/`, which Obsidian hides. Nothing is deleted. |
| Secrets | API keys, tokens, private keys and passwords are replaced with `[secret]` before a note is written, like every other scope. This matters more here: a vault is often synced to a cloud service. |

`folder` must be a relative path inside the vault (`Brokspace`,
`Work/Brokspace`): no `..`, no absolute path, no hidden folder.

## Searching the whole vault

Off by default. To let search, `brok ask` and your agents read every note of the vault:

```sh
brok config set memory.obsidian.search vault
```

```yaml
memory:
  obsidian:
    vault: ~/Documents/Obsidian/Main
    search: vault                    # folder (default) or vault
    exclude: [Private, "Journal/2024"]
```

- **Read only.** brok writes nowhere but `folder`. `memory list` and `memory forget` keep working on memories, not on your other notes.
- **Skipped:** hidden folders (`.obsidian/`, `.trash/`), everything in `exclude` (folders, files or globs such as `*.excalidraw.md`, relative to the vault), symbolic links, files that are not Markdown and notes larger than `index.max_file_kb`.
- **Found by:** the file name (spaces and accents included), the text, the `tags` and `aliases` properties, inline `#tags` and `[[wikilinks]]`. Results show the note's full path, so you can open it.
- **Secrets** in notes are replaced with `[secret]` in results, which may be sent to your LLM. Still, exclude folders that you do not want an LLM to see.
- **Size budget:** up to 20,000 notes or 64 MB of text, 8,000 characters per note; brok logs a warning when a vault is larger. A generated vault of 10,000 notes (40 MB) answered `brok memory search` in under a second using about 50 MB of memory on a laptop.
- Vault notes are ranked together with your memories.

## Cloud vaults

Obsidian has no network API for notes: a "cloud vault" is a **local folder
that a sync client keeps up to date**. brok works with that folder, so every
sync method works the same way: point `memory.obsidian.vault` at the local
copy.

| How the vault syncs | What to point `vault` at | Notes |
|---|---|---|
| **Obsidian Sync** (official, paid) | The vault folder on this machine | Notes brok writes are uploaded when the Obsidian app syncs. Keep Obsidian open, or open it before you expect the notes on another device. |
| **iCloud Drive** | `~/Library/Mobile Documents/iCloud~md~obsidian/Documents/<vault>` (macOS) | Make sure the folder is downloaded ("Keep downloaded"), or notes may be placeholders. |
| **Dropbox, OneDrive, Google Drive** (desktop clients) | The vault folder inside the synced directory | Wait for the client to finish the first sync before you rely on it. |
| **Syncthing** | The vault folder | Peer to peer, no cloud account. |
| **Git** (for example the Obsidian Git plugin) | The vault repository | Commit and push as you do for code. A vault in a team repository is a shared second brain. |

Things to know:

- **A missing vault is an error, not a new folder.** If the sync folder is on
  a drive that is not mounted (or the client is signed out), brok refuses to
  save with `no Obsidian vault: ... (is the vault's sync folder mounted?)`.
  Reading is the same: its notes are simply not searched until it is back.
- **Conflicts** are resolved by your sync tool. Two devices editing the same
  note produce a conflict copy; brok treats it as one more note. brok writes
  each memory to its own file (a new id each time), so conflicts only happen
  when you edit the same note in two places.
- **Windows and WSL**: use the Windows path in `brok.yml` when brok runs on
  Windows (`C:/Users/you/Obsidian/Main`), and `/mnt/c/Users/you/Obsidian/Main`
  when it runs in WSL. Avoid putting the vault inside a Linux-only path
  Obsidian on Windows cannot see.
- **Remote vaults without a local copy** (for example through the community
  Local REST API plugin) are not supported yet.

## A second brain with your coding agents

Your agents read and write memories over MCP (`brok_memory_save`,
`brok_memory_search`, `brok_memory_recent`). The `scope` argument accepts
`obsidian` once a vault is configured, so an agent can save a decision
straight into your second brain. Tell it when to do that, for example in
`AGENTS.md` or `CLAUDE.md`:

```markdown
When we settle an architecture decision, save it with `brok_memory_save`
using `scope: "obsidian"` and `kind: "decision"`. Keep everything else
in the default private scope.
```

Practical ways to use the vault together with Brokspace:

- **Decision log.** Decisions the agents and you take land in
  `Brokspace/` with `kind: decision`. A Dataview table keeps a living log:

  ````markdown
  ```dataview
  TABLE kind, tags, updated
  FROM "Brokspace"
  WHERE kind = "decision"
  SORT updated DESC
  ```
  ````

- **Link ideas to memories.** Add `[[wikilinks]]` in a note's body, or link
  to a Brokspace note from a daily note. Obsidian's backlinks and graph show
  how decisions relate. brok indexes the note's text, links included, so a
  search for the linked name finds the memory.
- **Hand-written notes count.** A note you write in `Brokspace/` (with or
  without front matter) is searchable by `brok search`, `brok ask` and your
  agents. Without front matter, its first heading becomes its title.
- **Handoffs between tools.** End a session with
  `brok memory add "Where I stopped" --kind handoff --scope obsidian`, then
  pick it up from the phone or another machine.
- **One vault, many workspaces.** Every workspace can point at the same
  vault. Use a different `folder` per workspace (`Brokspace/shop`,
  `Brokspace/payments`) to keep them apart, or one folder to share.

## Configuration

```yaml
memory:
  obsidian:
    vault: ~/Documents/Obsidian/Main
    folder: Brokspace
```

| Key | Default | Description |
|---|---|---|
| `memory.obsidian.vault` | unset (off) | The vault folder. Must exist. |
| `memory.obsidian.folder` | `Brokspace` | Folder inside the vault for brok's notes. |
| `memory.obsidian.search` | `folder` | `folder`: search only that folder. `vault`: search every note, read only. |
| `memory.obsidian.exclude` | `[]` | Folders, files or globs of the vault never searched in `vault` mode. |

Memory must be on (`memory.enabled: true`, the default). See
[memory.md](memory.md) for the other scopes and [configuration.md](configuration.md)
for every key.

## Not yet

These are not implemented; the first is the most requested next step:

- Writing notes with readable file names and wikilinks to related memories.
- Remote vaults through an Obsidian REST API plugin, for a vault that is
  not on this machine.
- An Obsidian plugin to browse `brok search` results from inside Obsidian.
