# Windows package managers: winget and Scoop

**For:** maintainers who release `brok`, `brok-agent` and Brokspace IDE.

> Brokspace IDE is discontinued (editor extensions replace it): skip its
> entries below. They stay for reference until the `ide` repository is archived.
**Time:** about 1 hour, once, plus waiting for the winget review.

Managed Windows machines block `irm | iex` (see
[code-signing.md](code-signing.md)). winget and Scoop give them an install
path their IT teams already allow.

| | Users run | What you set up |
|---|---|---|
| **winget** | `winget install Brokspace.Brok` | Manifests in `microsoft/winget-pkgs` + 1 secret |
| **Scoop** | `scoop bucket add brokspace https://github.com/brokspace/scoop-bucket` then `scoop install brok` | Repository `brokspace/scoop-bucket` |

Packages: `Brokspace.Brok`, `Brokspace.BrokAgent` (portable `.exe` from the
`brok` and `brok-agent` releases) and `Brokspace.BrokspaceIDE` (the NSIS
installer from the `ide` release).

**Before you start:** you are an owner of the `brokspace` organization,
`gh auth status` works, and the latest `brok`, `brok-agent` and `ide`
releases are published with their `.sha256` files.

---

## Part 1: Scoop bucket

### 1.1 Create the bucket

```sh
gh repo create brokspace/scoop-bucket --public \
  --description "Scoop bucket for Brokspace" --add-readme
git clone https://github.com/brokspace/scoop-bucket && cd scoop-bucket
mkdir bucket
```

### 1.2 Add the manifests

`bucket/brok.json` (the file is renamed to `brok.exe` on install):

```json
{
  "version": "0.4.0",
  "description": "Local-first workspace search, agents and skills for developers",
  "homepage": "https://github.com/brokspace/brok",
  "license": "<see the LICENSE>",
  "architecture": {
    "64bit": {
      "url": "https://github.com/brokspace/brok/releases/download/v0.4.0/brok-windows-x86_64.exe#/brok.exe",
      "hash": "<contents of brok-windows-x86_64.exe.sha256>"
    }
  },
  "bin": "brok.exe",
  "checkver": "github",
  "autoupdate": {
    "architecture": {
      "64bit": {
        "url": "https://github.com/brokspace/brok/releases/download/v$version/brok-windows-x86_64.exe#/brok.exe",
        "hash": { "url": "$url.sha256" }
      }
    }
  }
}
```

Do the same for `bucket/brok-agent.json` (repository `brokspace/brok-agent`,
asset `brok-agent-windows-x86_64.exe`), and add `"depends": "brok-agent"` to
`brok.json`. Check the license field against each repository's `LICENSE`.

`bucket/brokspace-ide.json` uses the NSIS installer:

```json
{
  "version": "0.3.0",
  "description": "Brokspace IDE",
  "homepage": "https://github.com/brokspace/ide",
  "license": "<see the ide LICENSE>",
  "architecture": {
    "64bit": {
      "url": "https://github.com/brokspace/ide/releases/download/v0.3.0/BrokspaceIDE-0.3.0-windows-x64-setup.exe",
      "hash": "<contents of the .sha256 file>"
    }
  },
  "innosetup": false,
  "installer": { "args": ["/S"] },
  "checkver": "github",
  "autoupdate": {
    "architecture": {
      "64bit": {
        "url": "https://github.com/brokspace/ide/releases/download/v$version/BrokspaceIDE-$version-windows-x64-setup.exe",
        "hash": { "url": "$url.sha256" }
      }
    }
  }
}
```

The IDE is a GUI installer, so a plain Scoop manifest is awkward: if it does
not install cleanly, ship only `brok` and `brok-agent` in the bucket and
leave the IDE to winget.

### 1.3 Keep it updated

Add `.github/workflows/excavator.yml` using the
[ScoopInstaller/GithubActions](https://github.com/ScoopInstaller/GithubActions)
excavator action on a daily schedule. It reads `checkver` and `autoupdate`
and commits new versions.

### 1.4 Verify (on Windows)

```powershell
scoop bucket add brokspace https://github.com/brokspace/scoop-bucket
scoop install brok
brok --version
brok-agent --version
```

---

## Part 2: winget

### 2.1 Create the manifests

Install [wingetcreate](https://github.com/microsoft/winget-create) and run,
once per package:

```powershell
winget install Microsoft.WingetCreate
wingetcreate new https://github.com/brokspace/brok/releases/download/v0.4.0/brok-windows-x86_64.exe
```

Use package identifiers `Brokspace.Brok`, `Brokspace.BrokAgent` and
`Brokspace.BrokspaceIDE`. For the CLI tools choose installer type
`portable` and command alias `brok` / `brok-agent`. For the IDE choose `nullsoft`
(silent switch `/S`). Before submitting `Brokspace.Brok`, add an entry under
`Dependencies.PackageDependencies` in its installer manifest with
`PackageIdentifier: Brokspace.BrokAgent` and `MinimumVersion` set to the oldest
agent version compatible with this brok release. Submit the agent package first
so the dependency is available. After installing Brok, verify both
`brok --version` and `brok-agent --version`. The command offers to submit the
pull request.

### 2.2 Validate before submitting

```powershell
winget validate --manifest <manifest folder>
winget settings --enable LocalManifestFiles
winget install --manifest <manifest folder>
```

Antivirus or SmartScreen may flag the unsigned binaries during the winget
review. Signing ([code-signing.md](code-signing.md)) makes approval
smoother, but is not required to submit.

### 2.3 Submit and wait

Submit the pull request to `microsoft/winget-pkgs`. A bot validates it and a
moderator reviews it, usually within a few days. Fix anything they flag in the
same PR.

### 2.4 Automate updates

1. Create a **classic** personal access token with the `public_repo` and `workflow` scopes
   (GitHub > Settings > Developer settings > Personal access tokens >
   Tokens (classic)). It must belong to an account with a fork of
   `microsoft/winget-pkgs`; fork it first.
2. Add it as the secret `WINGET_TOKEN` in `brokspace/brok`, `brokspace/brok-agent`
   and `brokspace/ide`.
3. Add a job after `publish` in each `release.yml`:

```yaml
  winget:
    needs: publish
    runs-on: windows-latest
    steps:
      - uses: vedantmgoyal9/winget-releaser@main
        with:
          release-tag: ${{ inputs.tag || github.ref_name }}
          identifier: Brokspace.Brok   # Brokspace.BrokAgent / Brokspace.BrokspaceIDE
          installers-regex: 'windows-x86_64\.exe$'   # IDE: 'windows-x64-setup\.exe$'
          fork-user: <fork-owner> # Account used in step 1
          token: ${{ secrets.WINGET_TOKEN }}
```

Pin the action to a commit SHA, as the rest of the workflows do. The first
version must already exist in `winget-pkgs`; the action only updates.

### 2.5 Verify

```powershell
winget install Brokspace.Brok
brok --version
```

---

## After setup

- Update each README install section with the new commands.
- Rotate `WINGET_TOKEN` when it expires; put the date in your calendar.
- Tick the boxes in the tracking issues (`brokspace/brok` and `brokspace/ide`).
