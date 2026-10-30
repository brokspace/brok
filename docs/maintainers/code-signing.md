# Code signing and trusted installs: plan

**Status:** planned, not started. **For:** maintainers who cut releases.

Today our release binaries are unsigned. That costs us users on exactly the
machines Brokspace is built for: company laptops. This document is the plan
to fix it, as checklists. Work through the phases in order; each one is
useful on its own.

## Why

What we saw while walking the getting started guide on a managed Windows 11
laptop:

- `irm https://.../install.ps1 | iex` was blocked by antivirus (AMSI), so
  the one-line install did not work. People had to download the `.exe`
  files by hand, check the `.sha256` files, and edit `PATH`.
- Unsigned executables get SmartScreen warnings, and many companies block
  unsigned software entirely (application control policies).
- On macOS, a binary downloaded with a browser is quarantined and Gatekeeper
  refuses to run it unless it is signed and notarized. (`curl` downloads, as
  in `install.sh`, are not quarantined, which is why the script works.)

## What signing fixes, and what it does not

| Problem | Fixed by |
|---|---|
| SmartScreen "unknown publisher" warning on `brok.exe` | Authenticode signing (phase 1). The warning fades as the signing identity builds reputation over releases; no certificate removes it on day one any more (EV lost that in 2024). |
| Companies that allow only signed software | Authenticode signing (phase 1) |
| `irm \| iex` blocked by antivirus | **Not signing.** `iex` runs a string, and signatures only apply to files. Fix: a package manager companies allow (winget, phase 2), and a "download, then run" path for a signed `install.ps1` (phase 1). |
| macOS Gatekeeper blocks browser downloads | Developer ID signing + notarization (phase 3) |
| "Is this binary really from your CI?" | Build provenance attestations (phase 0), free |

## Decisions to make first

- [ ] **Windows signing provider.** Options, cheapest first:
  - **SignPath Foundation** (recommended to try first): free for open
    source. Requirements: OSI license (we are MIT/Apache-2.0), public
    repositories, MFA for every team member, and we sign only what we build
    from our own source. The certificate is issued to SignPath Foundation,
    so the publisher shown is "SignPath Foundation", not "Brokspace". Apply
    at https://signpath.org/apply.html; their GitHub Actions integration
    may need to be requested.
  - **Azure Artifact Signing** (formerly Trusted Signing): about
    US$9.99/month, cloud-based, built for CI. Available to organizations in
    the USA, Canada, the EU and the UK, and to individual developers in the
    USA and Canada only. Billing starts when the account is created, even if
    identity validation fails. The publisher shown is our organization.
  - **OV certificate** from a CA (DigiCert, Sectigo, GlobalSign): about
    US$150-300/year, available worldwide; the key must live on a hardware
    token or cloud HSM, which makes CI signing harder.
  - Not EV: it costs more and, since 2024, gets the same SmartScreen
    treatment as OV.
- [ ] **Who is the publisher?** If we want "Brokspace" to appear as the
  publisher, we need a legal entity eligible for Artifact Signing or an OV
  certificate. Otherwise SignPath Foundation.
- [ ] **macOS:** join the Apple Developer Program (US$99/year), as an
  organization (needs a D-U-N-S number) or as an individual.
- [ ] **Where the secrets live:** GitHub organization secrets, limited to
  the `brok` and `brok-agent` repositories, used only by the `Release`
  workflows on tags. Prefer OIDC (Azure) over long-lived keys where possible.

## Phase 0: provenance attestations (free, no decisions needed)

Lets anyone check that a binary was built by our release workflow from a
given commit.

- [ ] In `brok` and `brok-agent` `.github/workflows/release.yml`, after
  building each asset, add `actions/attest-build-provenance` for the asset
  (needs `permissions: id-token: write, attestations: write`).
- [ ] Document the check in `docs/getting-started.md` (manual install path):
  `gh attestation verify brok-windows-x86_64.exe --repo brokspace/brok`.
- [ ] Optional later: `brok upgrade` verifies the attestation when `gh` is
  available.

**Done when:** `gh attestation verify` succeeds for every asset of a new
release.

## Phase 1: sign the Windows binaries and the install script

- [ ] Get the signing identity (see the decision above) and store its
  credentials as organization secrets.
- [ ] In the `windows-x86_64` job of both release workflows, sign the
  executable **before** computing its `.sha256` and uploading it. Order:
  build, sign, checksum, upload, attest.
- [ ] Use an RFC 3161 timestamp server, so signatures stay valid after the
  certificate expires.
- [ ] Sign `scripts/install.ps1` with the same identity (Authenticode) and
  attach the signed file to each release, so companies with an `AllSigned`
  or `RemoteSigned` policy can run it as a file.
- [ ] Add the "download, then run" path to the README and
  `docs/getting-started.md`:
  ```powershell
  Invoke-WebRequest https://github.com/brokspace/brok/releases/latest/download/install.ps1 -OutFile install.ps1
  Get-AuthenticodeSignature .\install.ps1   # Status: Valid
  .\install.ps1
  ```
- [ ] CI check after signing: `Get-AuthenticodeSignature` reports `Valid`
  for `brok.exe`, `brok-agent.exe` and `install.ps1`; fail the release if not.
- [ ] Optional: `brok upgrade` on Windows refuses a downloaded binary whose
  signature is not valid or not from our publisher (on top of the checksum).

**Done when:** on a clean Windows machine, the release `.exe` files show our
publisher (or SignPath Foundation) in Properties > Digital Signatures, and
`install.ps1` runs under `Set-ExecutionPolicy AllSigned`.

## Phase 2: winget package

Many companies allow `winget` even where downloaded scripts are blocked, and
winget installs, upgrades and uninstalls cleanly.

- [ ] Add a zip asset per Windows release, `brok-windows-x86_64.zip`,
  holding the signed `brok.exe` and `brok-agent.exe` (they must sit side by
  side: `brok` looks for the agent next to itself).
- [ ] Create the package `Brokspace.Brok` with `wingetcreate new` (installer
  type `zip`, nested installer type `portable`, both executables as nested
  files, `ArchiveBinariesDependOnPath: true` so the install folder goes on
  `PATH` and the two files stay together), validate with `winget validate`
  and test with `winget install -m <manifest folder>`, then submit the pull
  request to `microsoft/winget-pkgs` (one package version per PR).
- [ ] Automate updates: after each release, run `wingetcreate update
  Brokspace.Brok -u <zip url> -v <version> --submit` from the release
  workflow (needs a GitHub token able to open PRs on winget-pkgs).
- [ ] `brok upgrade`: detect a winget install (the executable lives under
  `%LOCALAPPDATA%\Microsoft\WinGet\Packages\`) and print
  `winget upgrade Brokspace.Brok` instead of replacing files, like it already
  does for Cargo installs.
- [ ] Docs: make `winget install Brokspace.Brok` the first Windows install
  option in the README and the getting started guide.

**Done when:** `winget install Brokspace.Brok` then `brok --version` works in
a new terminal, and `winget upgrade` picks up the next release.

## Phase 3: macOS signing and notarization

- [ ] Create a "Developer ID Application" certificate and an App Store
  Connect API key for notarization; store both as organization secrets.
- [ ] In the two macOS jobs of both release workflows: import the
  certificate into a temporary keychain, sign with
  `codesign --options runtime --timestamp`, zip the binary, submit it with
  `xcrun notarytool submit --wait`, and fail unless the status is
  `Accepted`. Then checksum, upload, attest.
- [ ] Note for the docs: a bare command line binary cannot have the
  notarization ticket stapled; Gatekeeper checks it online on first run.
- [ ] Optional: a Homebrew tap (`brew install brokspace/tap/brok`), the
  install path macOS developers expect.

**Done when:** a binary downloaded with Safari runs without a Gatekeeper
prompt (`spctl --assess --type execute` accepts it).

## Phase 4: Linux

Nothing blocks unsigned binaries on Linux. Phase 0 attestations cover
"where did this come from". Revisit only if users ask for distribution
packages (`.deb`, `.rpm`), which have their own signing.

## After each phase

- [ ] Update `CONTRIBUTING.md` (Releasing) with any new manual step.
- [ ] Add a CHANGELOG entry so users see it in `brok upgrade`.
- [ ] Re-check prices and eligibility before paying for anything: they
  change (the figures above were checked in October 2026).

## References

- Microsoft: [Code signing options for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
- [Azure Artifact Signing](https://learn.microsoft.com/en-us/azure/trusted-signing/)
- [SignPath Foundation](https://signpath.org/) and its [terms](https://signpath.org/terms)
- winget: [Create your package manifest](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest), [installer schema](https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.12.0/installer.md) (portable packages since winget 1.3)
- GitHub: [Using artifact attestations to establish provenance for builds](https://docs.github.com/actions/security-guides/using-artifact-attestations-to-establish-provenance-for-builds)
