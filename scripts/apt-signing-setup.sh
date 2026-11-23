#!/usr/bin/env bash
# One-time setup of the APT signing key for brokspace/brok.
#
# Generates a dedicated Ed25519 signing key in a throwaway GPG home, stores the
# armored private key as the APT_GPG_PRIVATE_KEY Actions secret and the key id
# as the APT_GPG_KEY_ID Actions variable, then deletes the key material from
# this machine. The private key is never printed. Needs `gh` authenticated with
# admin rights on the repository, and `gpg`.
#
#   scripts/apt-signing-setup.sh [owner/repo]     (default: brokspace/brok)
set -euo pipefail

repo="${1:-brokspace/brok}"
home="$(mktemp -d)"
trap 'gpgconf --homedir "$home" --kill all 2>/dev/null || true; rm -rf "$home"' EXIT
chmod 700 "$home"

if gh secret list --repo "$repo" | grep -q '^APT_GPG_PRIVATE_KEY'; then
  echo "APT_GPG_PRIVATE_KEY already exists in $repo; refusing to replace the signing key." >&2
  echo "Rotating the key means every user must install the new public key: do it deliberately." >&2
  exit 1
fi

gpg --homedir "$home" --batch --pinentry-mode loopback --passphrase '' \
  --quick-generate-key "Brokspace APT Archive <apt@brokspace.org>" ed25519 sign 3y

key_id="$(gpg --homedir "$home" --batch --list-secret-keys --with-colons | awk -F: '/^fpr:/ {print $10; exit}')"
[ -n "$key_id" ] || { echo "could not read the new key id" >&2; exit 1; }

gpg --homedir "$home" --batch --pinentry-mode loopback --passphrase '' --armor --export-secret-keys "$key_id" \
  | gh secret set APT_GPG_PRIVATE_KEY --repo "$repo"
gh variable set APT_GPG_KEY_ID --repo "$repo" --body "$key_id"

echo "Stored APT_GPG_PRIVATE_KEY (secret) and APT_GPG_KEY_ID=$key_id (variable) in $repo."
echo "The key expires in 3 years: put a reminder to rotate it. Back up nothing locally; the secret is the only copy."
echo "Next: enable GitHub Pages (Source: GitHub Actions), point the apt CNAME at brokspace.github.io, then run the Publish APT repository workflow."
