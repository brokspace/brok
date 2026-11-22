# Brokspace release and package publishing

Brokspace ships through independent release streams. Keep the CLI, agent and VS Code extension versions independent; each repository uses its own `vX.Y.Z` tag matching its manifest version.

## Distribution channels

| Channel | Source | Automation |
|---|---|---|
| GitHub Releases | Versioned standalone binaries, SHA-256 sidecars and Debian packages | Tag-triggered release workflow in each repository |
| crates.io | `brok-cli` workspace crates and `brok-agent` | Tag-triggered Trusted Publishing OIDC workflows |
| Homebrew | `brokspace/homebrew-brok` tap | Manual formula update workflow in `brokspace/brok` |
| APT | `https://apt.brokspace.org` | Manual signed GitHub Pages deployment from `brokspace/brok` |
| Visual Studio Marketplace | Publisher `brokspace`, extension `brokspace` | VS Code tag-triggered OIDC workflow |
| Open VSX | Namespace `brokspace` (Cursor, Windsurf, VSCodium) | Same VS Code workflow, with the `OVSX_PAT` secret |

## crates.io Trusted Publishing

The main workspace workflow is `.github/workflows/publish-crates.yml`. Configure a Trusted Publisher separately on each of these crates: `brok-core`, `brok-bus`, `brok-index`, `brok-shard`, `brok-skills`, and `brok-cli`.

For each crate, use:

- GitHub owner: `brokspace`
- Repository: `brok`
- Workflow file: `publish-crates.yml`
- Environment: `crates-io`

In `brokspace/brok`, create an Actions environment named `crates-io`, restricted to release tags (`v*`). The workflow uses `rust-lang/crates-io-auth-action@v1` to obtain a short-lived token and publishes the crates in dependency order, skipping versions already present.

Both crate workflows start on the release tag but wait (up to 90 minutes) for the Release workflow to publish the GitHub release, so nothing reaches crates.io for a release whose binaries failed to build. Re-run one by hand with `gh workflow run publish-crates.yml -f tag=vX.Y.Z` (`publish-crate.yml` in `brok-agent`).

For `brok-agent`, configure its crate's Trusted Publisher using owner `brokspace`, repository `brok-agent`, workflow `publish-crate.yml`, and environment `crates-io`. Create the matching environment in that repository.

A crate must already exist before its Trusted Publisher can be configured. If a crate has never been published, publish its first version once from a secure maintainer machine with a tightly scoped token, revoke that token immediately, and then configure OIDC for future versions. Do not put tokens in Git, workflow YAML or repository docs.

Official guidance: https://crates.io/docs/trusted-publishing

## Homebrew tap

Create a public repository named `brokspace/homebrew-brok`. Seed it with the `homebrew-brok/README.md` file from the publishing kit.

Create a fine-grained token scoped only to `brokspace/homebrew-brok`, with Contents read/write and Pull requests read/write permissions. Store it as `HOMEBREW_TAP_TOKEN` in the `brokspace/brok` Actions secrets.

After both Rust GitHub Releases are published, run **Actions → Update Homebrew tap** in `brokspace/brok` and supply the `brok_tag` and `agent_tag` independently. The workflow obtains the checksums from each `.sha256` release sidecar, generates `Formula/brok.rb` and `Formula/brok-agent.rb`, and opens a pull request in the tap. Review and merge that PR.

Install commands:

```sh
brew install brokspace/brok/brok
brew install brokspace/brok/brok-agent
```

The formulae use upstream standalone executables, not source builds. Linux x86_64 requires glibc 2.31 or newer; Linux ARM64 requires glibc 2.39 or newer.

## APT repository

The APT publisher runs from `.github/workflows/publish-apt.yml` and is intentionally manual. It selects the latest stable release from each Rust repository and requires `.deb` artifacts from both.

One-time setup:

1. Generate a dedicated signing key for the Brokspace APT archive.
2. Add the armored private key as `APT_GPG_PRIVATE_KEY` in `brokspace/brok` Actions secrets. If protected by a passphrase, add `APT_GPG_PASSPHRASE` as a secret.
3. Add the full signing key ID as the `APT_GPG_KEY_ID` Actions variable.
4. Configure repository Settings → Pages to use GitHub Actions.
5. Configure the DNS CNAME `apt` to `brokspace.github.io`, set the Pages custom domain to `apt.brokspace.org`, and enable HTTPS when available.

After both Rust releases contain `.deb` files, run **Actions → Publish APT repository**. It creates `Packages` indexes for `amd64` and `arm64`, signs `InRelease` and `Release.gpg`, exports the public key and deploys the static repository to Pages. The private key remains in GitHub Actions secrets and is not deployed.

Client setup:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://apt.brokspace.org/brokspace-archive-keyring.asc \
  | sudo gpg --dearmor -o /etc/apt/keyrings/brokspace.gpg
echo "deb [arch=amd64,arm64 signed-by=/etc/apt/keyrings/brokspace.gpg] https://apt.brokspace.org stable main" \
  | sudo tee /etc/apt/sources.list.d/brokspace.list
sudo apt update
sudo apt install brok brok-agent
```

## Visual Studio Marketplace

Register or verify publisher ID `brokspace` at https://marketplace.visualstudio.com/manage/publishers/.

Create a Marketplace Trusted Publishing policy with:

- Repository owner: `brokspace`
- Repository: `vscode`
- Workflow: `release.yml`
- GitHub Actions environment: `marketplace-production`

Create the `marketplace-production` environment in `brokspace/vscode`. The release workflow grants `id-token: write` only to its release job and invokes `@vscode/vsce` with `publish --oidc`; no `VSCE_PAT` is needed. The workflow uses `@vscode/vsce@4.0.2`, which supports Trusted Publishing.

Official guidance: https://code.visualstudio.com/api/working-with-extensions/publishing-extension

## Release runbook

### CLI

1. Update the workspace and crate versions, internal version constraints, and `CHANGELOG.md`.
2. Run the full CI and promote the intended changes to the release branch.
3. Push a tag matching the workspace version, such as `v0.4.2`.
4. Verify the GitHub Release has binaries, checksum sidecars and Linux `.deb` files. The crates.io workflow runs independently for the same tag.

### Agent

1. Update `Cargo.toml`, `Cargo.lock`, shard manifest metadata and `CHANGELOG.md` as appropriate.
2. Run CI and promote the release changes.
3. Push the matching agent tag, such as `v0.2.2`.
4. Verify the release assets and the automatic `shard.json` binary pinning update. The crates.io workflow runs independently for the same tag.

### Homebrew and APT

1. After both Rust releases are complete, update and review the Homebrew tap formula PR.
2. Run the APT publishing workflow and verify `InRelease` and the public key are reachable over HTTPS.
3. Test installation and upgrades on clean Ubuntu/Debian `amd64` and `arm64` environments.

### VS Code

1. Update `package.json`, `CHANGELOG.md`, localized strings and README as needed.
2. Run lint, typecheck, localization validation, build and VSIX packaging.
3. Push a tag matching the extension version, e.g. `v0.1.1`.
4. Verify Marketplace OIDC publishing succeeds and the VSIX is attached to the GitHub Release.

## Security and compatibility

- Protect stable branches and release tags; only trusted maintainers should be able to create `v*` tags.
- Use GitHub environment approval rules for `crates-io`, `marketplace-production`, and `github-pages` as appropriate.
- Keep APT private signing material and the Homebrew tap write token in Actions secrets only.
- Verify Linux glibc symbol checks in the release jobs; the configured Debian package floors are conservative compatibility limits.
- Test package upgrades before publishing a production release. APT currently indexes the latest stable release per crate stream, while historical `.deb` files remain on GitHub Releases.
