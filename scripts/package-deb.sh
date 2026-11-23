#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "Usage: $0 <package> <version> <architecture> <binary-path> <depends>" >&2
  exit 2
fi

package="$1"
version="$2"
architecture="$3"
binary="$4"
depends="$5"

case "$package" in
  brok|brok-agent) ;;
  *) echo "Unsupported package name: $package" >&2; exit 2 ;;
esac
case "$architecture" in
  amd64|arm64) ;;
  *) echo "Unsupported Debian architecture: $architecture" >&2; exit 2 ;;
esac
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+~][A-Za-z0-9.+:~_-]+)?$ ]] || {
  echo "Invalid Debian package version: $version" >&2
  exit 2
}
[[ -f "$binary" ]] || { echo "Binary not found: $binary" >&2; exit 1; }
[[ -f LICENSE-MIT && -f LICENSE-APACHE ]] || {
  echo "Run this script from the repository root containing LICENSE-MIT and LICENSE-APACHE" >&2
  exit 1
}
command -v dpkg-deb >/dev/null || { echo "dpkg-deb is required" >&2; exit 1; }

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
mkdir -p "$root/DEBIAN" "$root/usr/share/doc/$package"
install -Dm0755 "$binary" "$root/usr/bin/$package"
install -Dm0644 LICENSE-MIT "$root/usr/share/doc/$package/LICENSE-MIT"
install -Dm0644 LICENSE-APACHE "$root/usr/share/doc/$package/LICENSE-APACHE"

{
  printf '%s\n' 'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/'
  printf '%s\n' 'Upstream-Name: Brokspace' 'Source: https://github.com/brokspace/brok' ''
  printf '%s\n' 'Files: *' 'Copyright: Brokspace contributors' 'License: MIT OR Apache-2.0'
  printf '%s\n' ' This package is dual-licensed. The complete license texts are included'
  printf '%s\n' ' in LICENSE-MIT and LICENSE-APACHE in this documentation directory.' ''
  printf '%s\n' 'License: MIT'
  sed 's/^/ /' LICENSE-MIT
  printf '\n%s\n' 'License: Apache-2.0'
  sed 's/^/ /' LICENSE-APACHE
} > "$root/usr/share/doc/$package/copyright"

cat > "$root/DEBIAN/control" <<CONTROL
Package: $package
Version: $version
Section: devel
Priority: optional
Architecture: $architecture
Maintainer: Brokspace contributors
Depends: $depends
Homepage: https://brokspace.org
Description: Local-first developer workspace search and agent tools
 Brokspace indexes and searches local workspaces and connects coding agents
 to workspace-aware search, memory, skills, and developer workflows.
CONTROL

# Debian policy: directories 0755, files 0644 (the binary stays 0755), whatever the umask.
find "$root" -type d -exec chmod 0755 {} +
chmod 0644 "$root/DEBIAN/control" "$root/usr/share/doc/$package/copyright"

output="${package}_${version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$root" "$output"
echo "Created $output"
