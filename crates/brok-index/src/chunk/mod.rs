//! Turning source files into retrieval units ("chunks").
//!
//! Chunking is never character based. Code in a supported language is cut
//! along its syntax tree so that a function, method, class or impl block is
//! one atomic chunk ([`ast`]). Markdown is cut along its headings and every
//! other text file along blank-line separated blocks ([`text`]).

#[cfg(feature = "ast")]
mod ast;
mod text;

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::language::Language;

/// Syntactic role of a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkKind {
    /// Free function.
    Function,
    /// Function inside a class, impl or trait.
    Method,
    /// Class or record.
    Class,
    /// Struct or union.
    Struct,
    /// Enum.
    Enum,
    /// Interface.
    Interface,
    /// Trait.
    Trait,
    /// Rust impl block (header only when its methods are chunked separately).
    Impl,
    /// Module or namespace.
    Module,
    /// Type declaration.
    Type,
    /// Top level code between definitions (imports, constants, ...).
    Preamble,
    /// Markdown section.
    Section,
    /// Generic text block.
    Block,
}

impl ChunkKind {
    /// Lowercase name, as stored in the index.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Module => "module",
            Self::Type => "type",
            Self::Preamble => "preamble",
            Self::Section => "section",
            Self::Block => "block",
        }
    }

    /// Parses [`ChunkKind::as_str`] output.
    pub fn parse(value: &str) -> Self {
        match value {
            "function" => Self::Function,
            "method" => Self::Method,
            "class" => Self::Class,
            "struct" => Self::Struct,
            "enum" => Self::Enum,
            "interface" => Self::Interface,
            "trait" => Self::Trait,
            "impl" => Self::Impl,
            "module" => Self::Module,
            "type" => Self::Type,
            "preamble" => Self::Preamble,
            "section" => Self::Section,
            _ => Self::Block,
        }
    }
}

impl fmt::Display for ChunkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A retrieval unit with the metadata needed to cite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Stable id derived from path, byte range and content.
    pub id: String,
    /// Workspace relative path (`/` separators).
    pub path: String,
    /// Language id.
    pub language: String,
    /// Syntactic kind.
    pub kind: ChunkKind,
    /// Symbol name (function, class, heading, ...).
    pub symbol: Option<String>,
    /// Enclosing symbol (class of a method, impl of a fn, parent heading).
    pub parent: Option<String>,
    /// Declaration line(s) without the body.
    pub signature: Option<String>,
    /// 1-based first line.
    pub start_line: u32,
    /// 1-based last line, inclusive.
    pub end_line: u32,
    /// Byte offset of the first byte.
    pub start_byte: u32,
    /// Byte offset one past the last byte.
    pub end_byte: u32,
    /// Hash of the whole file the chunk was cut from.
    pub file_hash: String,
    /// Source text.
    pub content: String,
}

impl Chunk {
    /// Text fed to the embedding model and BM25: metadata first, so that
    /// symbol names weigh in even when the body is long.
    pub fn embedding_text(&self) -> String {
        let mut text = String::with_capacity(self.content.len() + 128);
        text.push_str(&self.path);
        if let Some(parent) = &self.parent {
            text.push_str(" > ");
            text.push_str(parent);
        }
        if let Some(symbol) = &self.symbol {
            text.push_str(" > ");
            text.push_str(symbol);
        }
        text.push('\n');
        text.push_str(&self.content);
        text
    }
}

/// A chunk under construction, before ids and file metadata are attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub kind: ChunkKind,
    pub symbol: Option<String>,
    pub parent: Option<String>,
    pub signature: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Chunking parameters.
#[derive(Debug, Clone, Copy)]
pub struct ChunkOptions {
    /// Syntactic units longer than this are split.
    pub max_lines: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self { max_lines: 120 }
    }
}

/// Splits `source` (the content of `path`) into chunks.
pub fn chunk_file(path: &str, source: &str, file_hash: &str, options: ChunkOptions) -> Vec<Chunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    let language = Language::detect(std::path::Path::new(path));
    let pieces = pieces_for(language, source, options);
    let index = LineIndex::new(source);
    pieces
        .into_iter()
        .filter_map(|piece| {
            let content = source.get(piece.start_byte..piece.end_byte)?;
            if content.trim().is_empty() {
                return None;
            }
            let start_line = index.line_of(piece.start_byte);
            let end_line = index.line_of(piece.end_byte.saturating_sub(1).max(piece.start_byte));
            Some(Chunk {
                id: chunk_id(path, piece.start_byte, piece.end_byte, content),
                path: path.to_owned(),
                language: language.id.to_owned(),
                kind: piece.kind,
                symbol: piece.symbol,
                parent: piece.parent,
                signature: piece.signature,
                start_line,
                end_line,
                start_byte: u32::try_from(piece.start_byte).unwrap_or(u32::MAX),
                end_byte: u32::try_from(piece.end_byte).unwrap_or(u32::MAX),
                file_hash: file_hash.to_owned(),
                content: content.to_owned(),
            })
        })
        .collect()
}

fn pieces_for(language: Language, source: &str, options: ChunkOptions) -> Vec<Piece> {
    #[cfg(feature = "ast")]
    if let Some(ast_language) = language.ast
        && let Some(pieces) = ast::chunk(ast_language, source, options)
    {
        return pieces;
    }
    if language.markdown { text::chunk_markdown(source, options) } else { text::chunk_blocks(source, options) }
}

fn chunk_id(path: &str, start: usize, end: usize, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update(start.to_le_bytes());
    hasher.update(end.to_le_bytes());
    hasher.update(content.as_bytes());
    hex::encode(&hasher.finalize()[..12])
}

/// Maps byte offsets to 1-based line numbers.
pub(crate) struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(source: &str) -> Self {
        let starts = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1)).collect::<Vec<usize>>();
        Self { starts }
    }

    pub(crate) fn line_of(&self, byte: usize) -> u32 {
        let line = self.starts.partition_point(|&start| start <= byte);
        u32::try_from(line.max(1)).unwrap_or(u32::MAX)
    }

    #[cfg(test)]
    pub(crate) fn line_count(&self) -> usize {
        self.starts.len()
    }
}

/// Splits `[start, end)` into consecutive windows of at most `max_lines`
/// lines, cutting only at line boundaries.
pub(crate) fn split_lines(source: &str, start: usize, end: usize, max_lines: usize) -> Vec<(usize, usize)> {
    let mut windows = Vec::new();
    let mut window_start = start;
    let mut lines = 0;
    for (offset, _) in source[start..end].match_indices('\n') {
        lines += 1;
        if lines == max_lines {
            let cut = start + offset + 1;
            windows.push((window_start, cut));
            window_start = cut;
            lines = 0;
        }
    }
    if window_start < end {
        windows.push((window_start, end));
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index() {
        let index = LineIndex::new("a\nbb\n\nccc");
        assert_eq!(index.line_of(0), 1);
        assert_eq!(index.line_of(2), 2);
        assert_eq!(index.line_of(5), 3);
        assert_eq!(index.line_of(6), 4);
        assert_eq!(index.line_count(), 4);
    }

    #[test]
    fn split_lines_respects_boundaries() {
        let text = "1\n2\n3\n4\n5\n";
        let windows = split_lines(text, 0, text.len(), 2);
        let parts: Vec<&str> = windows.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(parts, vec!["1\n2\n", "3\n4\n", "5\n"]);
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        let a = chunk_file("a.md", "# A\n\ntext\n", "h", ChunkOptions::default());
        let b = chunk_file("a.md", "# A\n\ntext\n", "h", ChunkOptions::default());
        let c = chunk_file("b.md", "# A\n\ntext\n", "h", ChunkOptions::default());
        assert_eq!(a[0].id, b[0].id);
        assert_ne!(a[0].id, c[0].id);
    }

    #[test]
    fn empty_files_have_no_chunks() {
        assert!(chunk_file("a.rs", "  \n", "h", ChunkOptions::default()).is_empty());
    }
}
