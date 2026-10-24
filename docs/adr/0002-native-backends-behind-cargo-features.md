# 0002. Native backends behind default-on cargo features

- **Status**: Accepted
- **Date**: 2026-10-24

## Context

The best retrieval quality needs heavy native dependencies: tree-sitter grammars (C), LanceDB (protoc and a large dependency tree) and ONNX Runtime (a prebuilt native library). Requiring all of them makes contributing harder and blocks some platforms.

## Decision

`brok-index` (and `brok-cli`, forwarding them) exposes the native backends as cargo features, all enabled by default: `ast`, `lancedb`, `onnx`, plus the optional `cuda`, `coreml` and `directml`. Without them the build is pure Rust and still fully functional: structural text chunking, a memory-mapped Arrow IPC chunk store with exact vector search, and BM25 search. Runtime configuration (`index.store: auto`, `embeddings.enabled`) adapts to what was compiled in.

## Consequences

- `cargo test --no-default-features` works on any machine with a Rust toolchain, and CI tests both builds on Linux, macOS and Windows.
- Code must keep the lean path compiling: feature-gated modules, `cfg` on dispatch arms, tests that run in both modes.
- Released binaries use the default features.
