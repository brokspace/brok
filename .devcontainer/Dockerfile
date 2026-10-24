# Reproducible development environment for every Brokspace repository.
#
# Includes everything the native backends need (tree-sitter, LanceDB, ONNX
# Runtime) plus a MinGW cross toolchain to type-check the Windows code paths.
#
#   docker build -t brok-dev .devcontainer
#   docker run --rm -it -v "$PWD/..:/src" -w /src/brok brok-dev cargo test
FROM rust:1.99-trixie

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      protobuf-compiler libprotobuf-dev cmake clang libssl-dev pkg-config git git-lfs mingw-w64 \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt \
 && rustup target add x86_64-pc-windows-gnu

ENV CARGO_TERM_COLOR=always
