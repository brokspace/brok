# 0017. Store vectors as float16, keep 768 dimensions

- **Status**: Accepted
- **Date**: 2026-11-26

## Context

brokspace/brok#73 asks for smaller vector storage without material loss of search quality. Vectors are
stored as `Float32`, 768 per chunk, which is 3072 bytes. In a measured index (521 files, 3356 chunks,
Arrow store) the chunk store takes 13.7 MB, of which vectors are 10.3 MB (75%); the keyword index and
manifest add 1.0 MB.

## Measurements

`scripts/vector_study.py` on the sources of six Brokspace repositories, quantized model, exact search
over vectors only (the keyword half of hybrid search is left out). "Recall" is the share of the
`float32` 768 results that the format also returns, for 30 written questions (recall@8 and @32; brok
fetches four times the result limit) and for 500 chunks used as queries (recall@10). "Found" is the
number of questions whose answer file is in the top 8.

| storage | dims | bytes per vector | recall@8 | recall@32 | chunk recall@10 | found | MRR |
|---|---:|---:|---:|---:|---:|---:|---:|
| float32 | 768 | 3072 | 100% | 100% | 100% | 27/30 | 0.524 |
| float16 | 768 | 1536 | 100.0% | 99.9% | 100.0% | 27/30 | 0.524 |
| int8 | 768 | 772 | 98.8% | 99.4% | 99.0% | 27/30 | 0.526 |
| binary | 768 | 96 | 53.3% | 54.8% | 62.5% | 24/30 | 0.483 |
| float32 | 512 | 2048 | 80.0% | 82.4% | 87.1% | 22/30 | 0.538 |
| int8 | 512 | 516 | 80.8% | 82.4% | 87.1% | 23/30 | 0.539 |
| float32 | 256 | 1024 | 66.7% | 72.5% | 78.1% | 23/30 | 0.507 |
| int8 | 256 | 260 | 67.9% | 72.6% | 78.0% | 23/30 | 0.530 |
| float32 | 128 | 512 | 59.6% | 61.4% | 68.1% | 24/30 | 0.389 |

Binary vectors with the best 128 rescored by `int8` reach 86.1% recall@32, and need both copies.

For the measured index, the chunk store would shrink from 13.7 MB to about 8.5 MB with `float16`
(38% less) and 5.9 MB with `int8` (57% less). These two figures are computed from vector sizes, not
measured on written files.

## Decision

1. **New indexes store `float16` vectors** in both backends. No setting: for these questions it returned the same results as
   `float32` (99.9% at 32, 100% at 8) and halves vector storage.
2. **Dimensions stay at 768 by default.** Fewer dimensions change the results far more than a smaller
   number type does. `embeddings.dimensions` remains for those who accept that.
3. **`int8` is not adopted now.** It saves another quarter of the original vector size, but needs a
   scale per vector, its own scoring in the Arrow store and a check of LanceDB's indexed search.
   Revisit if users report that `float16` is still too large.
4. **Binary vectors are rejected**: recall falls by almost half without a second stored copy.
5. **Existing indexes keep working and are not rebuilt automatically.** A store keeps the type it was
   created with; `brok index --rebuild` converts it. Rebuilding embeds everything again, so upgrading
   must not force it.
6. **Older brok versions must not read a `float16` index as having no vectors.** The partition manifest
   version is raised so that an older brok rebuilds the partition instead.

## Measured on the implementation

`cargo test --release -p brok-index precision_benchmark -- --ignored --nocapture` writes both types with
each store and searches them (32 results, 50 queries, one Linux x86_64 machine with F16C). The 3356
chunk set is the real embeddings above; 10 000 and 100 000 are synthetic clustered vectors. Chunks carry
about 1.2 KB of text. Recall is against exact `float32` search over the same vectors.

| store | chunks | vectors | MB on disk | search p50 ms | p95 ms | recall@32 |
|---|---:|---|---:|---:|---:|---:|
| arrow | 3356 | float32 | 14.7 | 2.3 | 2.5 | 100% |
| arrow | 3356 | float16 | 9.6 | 2.5 | 2.8 | 99.9% |
| lancedb | 3356 | float32 | 10.4 | 3.6 | 4.2 | 100% |
| lancedb | 3356 | float16 | 5.3 | 3.4 | 4.2 | 99.9% |
| arrow | 10000 | float32 | 43.9 | 6.8 | 7.6 | 100% |
| arrow | 10000 | float16 | 28.6 | 7.5 | 9.0 | 99.9% |
| lancedb | 10000 | float32 | 63.2 | 13.4 | 15.5 | 100% |
| lancedb | 10000 | float16 | 32.1 | 11.6 | 12.9 | 100% |
| arrow | 100000 | float32 | 439.5 | 69.9 | 77.5 | 100% |
| arrow | 100000 | float16 | 285.9 | 76.5 | 85.2 | 99.9% |
| lancedb | 100000 | float32 | 632.6 | 107.2 | 123.4 | 100% |
| lancedb | 100000 | float16 | 325.0 | 72.4 | 86.6 | 99.9% |

The chunk store shrinks by about 35% (Arrow) and 49% (LanceDB). Arrow search is about 10% slower
(widening to `f32`), LanceDB search is the same or faster. A machine without hardware conversion
(older x86, some virtual machines) would be slower in the Arrow store; that was not measured.

While measuring, the LanceDB vector index turned out not to be used: it is built with the L2 metric
and searched with cosine, so LanceDB scanned every row. This predated this decision and affected both
types alike, so the LanceDB figures are exact search. Fixed afterwards (brokspace/brok#192,
[measurements](../benchmarks.md#vector-index)).

## Limits of the evidence

One small corpus of mostly Rust, TypeScript and documentation; 30 questions written by one person;
vector search alone. The script embeds questions with the Python ONNX Runtime, whose vectors for 16
stored chunks had a cosine of at least 0.987 with brok's.

## Consequences

- Half the vector storage for new indexes, with one more number type to read and a small pure Rust
  dependency (`half`).
- Two vector types exist in the wild until indexes are rebuilt.
- Downgrading brok after creating an index costs a rebuild.
