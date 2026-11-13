#!/usr/bin/env bash
# Builds the Linux x86_64 release binary so it runs on glibc 2.31 and newer
# (Ubuntu 20.04, Debian 11) and needs no libssl or libstdc++ from the system.
#
# Run it on Ubuntu 20.04 with gcc-13, g++-13, perl, protoc and libssl-dev
# installed (the release workflow does this). Extra arguments go to cargo.
#
#   scripts/build-linux-compat.sh
#
# Three things make the binary portable:
#   - the old build host pins glibc symbol versions at 2.31;
#   - libstdc++ is linked statically, because the prebuilt ONNX Runtime needs
#     a newer one than old distributions ship;
#   - OpenSSL is built in (feature `vendored-openssl`), instead of expecting
#     the libssl.so version of the machine it runs on.
# scripts/glibc-compat.c covers the few glibc symbols ONNX Runtime needs
# that glibc 2.31 lacks.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

export CC=gcc-13 CXX=g++-13
gcc-13 -O2 -c "$root/scripts/glibc-compat.c" -o "$work/glibc-compat.o"

# A directory holding only libstdc++.a, so that `-lstdc++` picks the static one.
mkdir "$work/static"
cp "$(g++-13 -print-file-name=libstdc++.a)" "$work/static/"

export RUSTFLAGS="-C linker=g++-13 -C link-arg=-L$work/static -C link-arg=-static-libgcc -C link-arg=$work/glibc-compat.o ${RUSTFLAGS:-}"

cd "$root"
cargo build --release -p brok-cli --target x86_64-unknown-linux-gnu --features vendored-openssl "$@"
