//! Times BM25 queries on synthetic corpora:
//! `cargo run --release -p brok-index --no-default-features --example bm25_topk`

use std::hint::black_box;
use std::time::Instant;

use brok_index::bm25::Bm25Index;

fn main() {
    println!("{:>9} {:>8} {:>4} {:>12}", "documents", "term", "k", "us/query");
    for documents in [1_000usize, 20_000, 200_000] {
        let mut index = Bm25Index::default();
        for i in 0..documents {
            // "shared" is in every chunk, "rare" in one of a thousand; lengths
            // and frequencies vary so that scores differ.
            let rare = if i % 1_000 == 0 { "rare " } else { "" };
            let text = format!("{}{rare}{}", "shared ".repeat(1 + i % 5), "filler ".repeat(i % 17));
            index.add(&format!("chunk{i}"), &format!("file{i}.rs"), &text);
        }
        for term in ["rare", "shared"] {
            for k in [10usize, 20, 100] {
                let rounds = (4_000_000 / documents).max(20);
                let started = Instant::now();
                for _ in 0..rounds {
                    black_box(index.search(black_box(term), k));
                }
                let micros = started.elapsed().as_secs_f64() * 1e6 / rounds as f64;
                println!("{documents:>9} {term:>8} {k:>4} {micros:>12.1}");
            }
        }
    }
}
