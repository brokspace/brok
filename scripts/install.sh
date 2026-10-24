#!/bin/sh
# Installs brok and brok-agent from GitHub Releases (Linux and macOS).
#
#   curl -fsSL https://raw.githubusercontent.com/brokspace/brok/main/scripts/install.sh | sh
#
# Environment overrides:
#   BROK_VERSION      release tag to install (default: latest)
#   BROK_INSTALL_DIR  destination directory (default: ~/.local/bin)
set -eu

VERSION="${BROK_VERSION:-latest}"
INSTALL_DIR="${BROK_INSTALL_DIR:-$HOME/.local/bin}"
BASE="https://github.com/brokspace"

say() { printf '%s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) fail "unsupported OS $(uname -s); on Windows use scripts/install.ps1" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail "unsupported architecture $(uname -m)" ;;
esac
platform="$os-$arch"

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
else
  fail "curl or wget is required"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$INSTALL_DIR"

for bin in brok brok-agent; do
  if [ "$VERSION" = latest ]; then
    url="$BASE/$bin/releases/latest/download/$bin-$platform"
  else
    url="$BASE/$bin/releases/download/$VERSION/$bin-$platform"
  fi
  say "• downloading $bin ($platform)"
  fetch "$url" "$tmp/$bin" || fail "could not download $url"
  fetch "$url.sha256" "$tmp/$bin.sha256" || fail "could not download the checksum for $bin"
  expected="$(cut -d' ' -f1 <"$tmp/$bin.sha256")"
  actual="$(sha256 "$tmp/$bin")"
  [ "$expected" = "$actual" ] || fail "checksum mismatch for $bin (expected $expected, got $actual)"
  chmod 755 "$tmp/$bin"
  mv "$tmp/$bin" "$INSTALL_DIR/$bin"
done

say "✓ installed brok and brok-agent into $INSTALL_DIR"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "  add it to your PATH:  export PATH=\"$INSTALL_DIR:\$PATH\"  (put it in your shell profile)" ;;
esac
say ""
say "Next, in your project:  brok init"
