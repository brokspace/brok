# 0017. Store vectors as float16, keep 768 dimensions

- **Status**: Proposed
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

## Before this is accepted

The implementation has to show, on both backends:

- search latency for `float16` against `float32`, at 10 000 and 100 000 chunks (not measured here; the
  Arrow store scores every vector, and LanceDB builds an index from 10 000 rows);
- file sizes actually written, including LanceDB's index and old versions;
- the same recall table from vectors read back from each store.

LanceDB 0.41 (lance 14) contains `Float16` and `Int8` distance functions; indexed search with them was
not tried.

## Limits of the evidence

One small corpus of mostly Rust, TypeScript and documentation; 30 questions written by one person;
vector search alone. The script embeds questions with the Python ONNX Runtime, whose vectors for 16
stored chunks had a cosine of at least 0.987 with brok's.

## Consequences

- Half the vector storage for new indexes, with one more number type to read and a small pure Rust
  dependency (`half`).
- Two vector types exist in the wild until indexes are rebuilt.
- Downgrading brok after creating an index costs a rebuild.
