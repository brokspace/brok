//! Structure-aware chunking for text without a syntax tree.

use super::{ChunkKind, ChunkOptions, Piece, split_lines};

/// Preferred size of a text block; long blocks still go up to `max_lines`.
const TARGET_LINES: usize = 40;

/// One Markdown section per heading (fenced code blocks are respected).
/// Oversized sections fall back to block chunking inside the section.
pub(super) fn chunk_markdown(source: &str, options: ChunkOptions) -> Vec<Piece> {
    let mut headings: Vec<(usize, usize, String)> = Vec::new(); // (byte, level, title)
    let mut in_fence = false;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence {
            let level = trimmed.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&level) && trimmed[level..].starts_with([' ', '\t']) {
                headings.push((offset, level, trimmed[level..].trim().to_owned()));
            }
        }
        offset += line.len();
    }

    let mut pieces = Vec::new();
    let mut stack: Vec<(usize, String)> = Vec::new(); // heading hierarchy for `parent`
    let first = headings.first().map_or(source.len(), |h| h.0);
    if first > 0 {
        pieces.extend(blocks(source, 0, first, options, ChunkKind::Section, None, None));
    }
    for (i, (start, level, title)) in headings.iter().enumerate() {
        let end = headings.get(i + 1).map_or(source.len(), |h| h.0);
        stack.retain(|(l, _)| l < level);
        let parent = stack.last().map(|(_, t)| t.clone());
        stack.push((*level, title.clone()));
        pieces.extend(blocks(source, *start, end, options, ChunkKind::Section, Some(title.clone()), parent));
    }
    pieces
}

/// Greedy blank-line block packing for arbitrary text.
pub(super) fn chunk_blocks(source: &str, options: ChunkOptions) -> Vec<Piece> {
    blocks(source, 0, source.len(), options, ChunkKind::Block, None, None)
}

/// Packs paragraphs of `[start, end)` into pieces of about [`TARGET_LINES`]
/// lines, never exceeding `max_lines`. A section that fits in `max_lines`
/// stays whole.
fn blocks(
    source: &str,
    start: usize,
    end: usize,
    options: ChunkOptions,
    kind: ChunkKind,
    symbol: Option<String>,
    parent: Option<String>,
) -> Vec<Piece> {
    let make = |s: usize, e: usize| Piece {
        kind,
        symbol: symbol.clone(),
        parent: parent.clone(),
        signature: None,
        start_byte: s,
        end_byte: e,
    };
    let text = &source[start..end];
    if text.lines().count() <= options.max_lines {
        return vec![make(start, end)];
    }

    let target = TARGET_LINES.min(options.max_lines);
    let mut pieces = Vec::new();
    let mut piece_start = start;
    let mut piece_lines = 0;
    let mut offset = start;
    let mut paragraph_lines = 0;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        paragraph_lines += 1;
        let paragraph_ends = line.trim().is_empty();
        if paragraph_ends {
            piece_lines += paragraph_lines;
            paragraph_lines = 0;
            if piece_lines >= target {
                pieces.push((piece_start, offset));
                piece_start = offset;
                piece_lines = 0;
            }
        }
    }
    if piece_start < end {
        pieces.push((piece_start, end));
    }

    pieces
        .into_iter()
        .flat_map(|(s, e)| split_lines(source, s, e, options.max_lines))
        .map(|(s, e)| make(s, e))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(max_lines: usize) -> ChunkOptions {
        ChunkOptions { max_lines }
    }

    #[test]
    fn markdown_sections_with_parents() {
        let md = "intro\n# Title\ntext\n## Install\n```sh\n# not a heading\n```\n## Usage\nrun it\n";
        let pieces = chunk_markdown(md, opts(120));
        let symbols: Vec<_> = pieces.iter().map(|p| p.symbol.as_deref()).collect();
        assert_eq!(symbols, vec![None, Some("Title"), Some("Install"), Some("Usage")]);
        assert_eq!(pieces[2].parent.as_deref(), Some("Title"));
        assert!(md[pieces[2].start_byte..pieces[2].end_byte].contains("# not a heading"));
    }

    #[test]
    fn long_text_is_split_at_paragraphs() {
        let paragraph = "line\n".repeat(30) + "\n";
        let text = paragraph.repeat(5);
        let pieces = chunk_blocks(&text, opts(100));
        assert!(pieces.len() >= 3);
        for p in &pieces {
            let chunk = &text[p.start_byte..p.end_byte];
            assert!(chunk.lines().count() <= 100);
        }
        assert_eq!(pieces.first().unwrap().start_byte, 0);
        assert_eq!(pieces.last().unwrap().end_byte, text.len());
    }

    #[test]
    fn huge_paragraphs_are_hard_split() {
        let text = "x\n".repeat(250);
        let pieces = chunk_blocks(&text, opts(100));
        assert_eq!(pieces.len(), 3);
    }
}
