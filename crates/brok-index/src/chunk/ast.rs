//! Syntax-aware chunking with tree-sitter.
//!
//! Rules, applied at the top level and recursively inside containers:
//!
//! 1. Every definition (function, method, class, struct, impl, ...) becomes
//!    one chunk, including the doc comments and attributes/decorators
//!    directly above it.
//! 2. A definition longer than `max_lines` that contains other definitions
//!    (a big class or impl block) emits a short *header* chunk with its
//!    signature and is then chunked member by member.
//! 3. A long definition without members is split at line boundaries; every
//!    part keeps the symbol and signature metadata.
//! 4. Code between definitions (imports, constants, statements) is grouped
//!    into `preamble` chunks (or `block` chunks inside containers).

use std::cell::RefCell;
use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use super::{ChunkKind, ChunkOptions, LineIndex, Piece, split_lines};
use crate::language::AstLanguage;

struct Spec {
    language: fn() -> tree_sitter::Language,
    definitions: &'static [(&'static str, ChunkKind)],
    /// Nodes wrapping a definition (decorators, `export`).
    wrappers: &'static [&'static str],
    /// Nodes attached to the following definition (comments, attributes).
    trivia: &'static [&'static str],
}

const RUST: Spec = Spec {
    language: || tree_sitter_rust::LANGUAGE.into(),
    definitions: &[
        ("function_item", ChunkKind::Function),
        ("function_signature_item", ChunkKind::Function),
        ("struct_item", ChunkKind::Struct),
        ("union_item", ChunkKind::Struct),
        ("enum_item", ChunkKind::Enum),
        ("trait_item", ChunkKind::Trait),
        ("impl_item", ChunkKind::Impl),
        ("mod_item", ChunkKind::Module),
        ("macro_definition", ChunkKind::Function),
    ],
    wrappers: &[],
    trivia: &["line_comment", "block_comment", "attribute_item"],
};

const PYTHON: Spec = Spec {
    language: || tree_sitter_python::LANGUAGE.into(),
    definitions: &[("function_definition", ChunkKind::Function), ("class_definition", ChunkKind::Class)],
    wrappers: &["decorated_definition"],
    trivia: &["comment"],
};

const JS_DEFINITIONS: &[(&str, ChunkKind)] = &[
    ("function_declaration", ChunkKind::Function),
    ("generator_function_declaration", ChunkKind::Function),
    ("class_declaration", ChunkKind::Class),
    ("method_definition", ChunkKind::Method),
];

const JAVASCRIPT: Spec = Spec {
    language: || tree_sitter_javascript::LANGUAGE.into(),
    definitions: JS_DEFINITIONS,
    wrappers: &["export_statement"],
    trivia: &["comment"],
};

const TS_DEFINITIONS: &[(&str, ChunkKind)] = &[
    ("function_declaration", ChunkKind::Function),
    ("generator_function_declaration", ChunkKind::Function),
    ("class_declaration", ChunkKind::Class),
    ("abstract_class_declaration", ChunkKind::Class),
    ("method_definition", ChunkKind::Method),
    ("interface_declaration", ChunkKind::Interface),
    ("enum_declaration", ChunkKind::Enum),
    ("type_alias_declaration", ChunkKind::Type),
    ("internal_module", ChunkKind::Module),
];

const TYPESCRIPT: Spec = Spec {
    language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    definitions: TS_DEFINITIONS,
    wrappers: &["export_statement"],
    trivia: &["comment"],
};

const TSX: Spec = Spec {
    language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    definitions: TS_DEFINITIONS,
    wrappers: &["export_statement"],
    trivia: &["comment"],
};

const GO: Spec = Spec {
    language: || tree_sitter_go::LANGUAGE.into(),
    definitions: &[
        ("function_declaration", ChunkKind::Function),
        ("method_declaration", ChunkKind::Method),
        ("type_declaration", ChunkKind::Type),
    ],
    wrappers: &[],
    trivia: &["comment"],
};

const JAVA: Spec = Spec {
    language: || tree_sitter_java::LANGUAGE.into(),
    definitions: &[
        ("class_declaration", ChunkKind::Class),
        ("record_declaration", ChunkKind::Class),
        ("interface_declaration", ChunkKind::Interface),
        ("annotation_type_declaration", ChunkKind::Interface),
        ("enum_declaration", ChunkKind::Enum),
        ("method_declaration", ChunkKind::Method),
        ("constructor_declaration", ChunkKind::Method),
    ],
    wrappers: &[],
    trivia: &["line_comment", "block_comment"],
};

fn spec(language: AstLanguage) -> &'static Spec {
    match language {
        AstLanguage::Rust => &RUST,
        AstLanguage::Python => &PYTHON,
        AstLanguage::JavaScript => &JAVASCRIPT,
        AstLanguage::TypeScript => &TYPESCRIPT,
        AstLanguage::Tsx => &TSX,
        AstLanguage::Go => &GO,
        AstLanguage::Java => &JAVA,
    }
}

thread_local! {
    /// Parsers are reused per thread: creating one per file is measurable.
    static PARSERS: RefCell<HashMap<AstLanguage, Parser>> = RefCell::new(HashMap::new());
}

/// Chunks `source`; `None` when the grammar cannot be loaded or parsing fails.
pub(super) fn chunk(language: AstLanguage, source: &str, options: ChunkOptions) -> Option<Vec<Piece>> {
    let spec = spec(language);
    let tree = PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        let parser = match parsers.entry(language) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut parser = Parser::new();
                if let Err(e) = parser.set_language(&(spec.language)()) {
                    tracing::warn!(?language, error = %e, "cannot load tree-sitter grammar");
                    return None;
                }
                entry.insert(parser)
            }
        };
        parser.parse(source, None)
    })?;

    let walker = Walker { spec, source, lines: LineIndex::new(source), max_lines: options.max_lines.max(1) };
    let mut pieces = Vec::new();
    walker.walk(tree.root_node(), None, &mut pieces);
    Some(pieces)
}

struct Walker<'s> {
    spec: &'static Spec,
    source: &'s str,
    lines: LineIndex,
    max_lines: usize,
}

/// A run of consecutive nodes that is not a definition.
#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    end_row: usize,
}

impl<'s> Walker<'s> {
    fn walk(&self, container: Node<'_>, parent: Option<&str>, out: &mut Vec<Piece>) {
        let mut loose: Option<Span> = None;
        let mut trivia: Option<Span> = None;
        let mut cursor = container.walk();
        let children: Vec<Node<'_>> = container.named_children(&mut cursor).collect();

        for child in children {
            let row = child.start_position().row;
            let child_span =
                Span { start: child.start_byte(), end: child.end_byte(), end_row: child.end_position().row };

            if self.spec.trivia.contains(&child.kind()) {
                trivia = match trivia {
                    // A blank line breaks the run: earlier trivia is not attached.
                    Some(run) if row > run.end_row + 1 => {
                        loose = Some(merge(loose, run));
                        Some(child_span)
                    }
                    Some(run) => Some(Span { end: child_span.end, end_row: child_span.end_row, ..run }),
                    None => Some(child_span),
                };
                continue;
            }

            if let Some((definition, kind)) = self.definition(child) {
                let start = match trivia.take() {
                    Some(run) if row <= run.end_row + 1 => run.start,
                    Some(run) => {
                        loose = Some(merge(loose, run));
                        child.start_byte()
                    }
                    None => child.start_byte(),
                };
                if let Some(span) = loose.take() {
                    self.emit_loose(span, parent, out);
                }
                self.emit_definition(start, child, definition, kind, parent, out);
            } else {
                let span = match trivia.take() {
                    Some(run) => merge(Some(run), child_span),
                    None => child_span,
                };
                loose = Some(merge(loose, span));
            }
        }

        if let Some(run) = trivia {
            loose = Some(merge(loose, run));
        }
        if let Some(span) = loose {
            self.emit_loose(span, parent, out);
        }
    }

    /// Returns the definition node and its kind when `node` is a definition
    /// or wraps one.
    fn definition<'t>(&self, node: Node<'t>) -> Option<(Node<'t>, ChunkKind)> {
        let kind_of = |n: Node<'_>| self.spec.definitions.iter().find(|(k, _)| *k == n.kind()).map(|(_, kind)| *kind);
        if let Some(kind) = kind_of(node) {
            return Some((node, kind));
        }
        if self.spec.wrappers.contains(&node.kind()) {
            let mut cursor = node.walk();
            return node.named_children(&mut cursor).find_map(|child| kind_of(child).map(|kind| (child, kind)));
        }
        None
    }

    fn emit_definition(
        &self,
        start: usize,
        outer: Node<'_>,
        definition: Node<'_>,
        kind: ChunkKind,
        parent: Option<&str>,
        out: &mut Vec<Piece>,
    ) {
        let end = outer.end_byte();
        let kind = if parent.is_some() && kind == ChunkKind::Function { ChunkKind::Method } else { kind };
        let symbol = self.symbol(definition);
        let signature = self.signature(definition);
        let piece = |start_byte, end_byte| Piece {
            kind,
            symbol: symbol.clone(),
            parent: parent.map(str::to_owned),
            signature: signature.clone(),
            start_byte,
            end_byte,
        };

        if self.line_span(start, end) <= self.max_lines {
            out.push(piece(start, end));
            return;
        }
        if let Some(body) = definition.child_by_field_name("body").filter(|body| self.has_definitions(*body)) {
            out.push(piece(start, body.start_byte()));
            let scope = symbol.as_deref().map_or_else(
                || parent.map(str::to_owned),
                |name| Some(parent.map_or_else(|| name.to_owned(), |p| format!("{p}::{name}"))),
            );
            self.walk(body, scope.as_deref(), out);
            return;
        }
        out.extend(split_lines(self.source, start, end, self.max_lines).into_iter().map(|(s, e)| piece(s, e)));
    }

    fn emit_loose(&self, span: Span, parent: Option<&str>, out: &mut Vec<Piece>) {
        let kind = if parent.is_some() { ChunkKind::Block } else { ChunkKind::Preamble };
        for (start, end) in split_lines(self.source, span.start, span.end, self.max_lines) {
            out.push(Piece {
                kind,
                symbol: None,
                parent: parent.map(str::to_owned),
                signature: None,
                start_byte: start,
                end_byte: end,
            });
        }
    }

    fn has_definitions(&self, node: Node<'_>) -> bool {
        let mut cursor = node.walk();
        node.named_children(&mut cursor).any(|child| self.definition(child).is_some())
    }

    fn line_span(&self, start: usize, end: usize) -> usize {
        let first = self.lines.line_of(start) as usize;
        let last = self.lines.line_of(end.saturating_sub(1).max(start)) as usize;
        last + 1 - first
    }

    fn text(&self, node: Node<'_>) -> &'s str {
        &self.source[node.byte_range()]
    }

    fn symbol(&self, node: Node<'_>) -> Option<String> {
        if node.kind() == "impl_item" {
            let ty = self.text(node.child_by_field_name("type")?);
            return Some(match node.child_by_field_name("trait") {
                Some(tr) => format!("{} for {ty}", self.text(tr)),
                None => ty.to_owned(),
            });
        }
        if let Some(name) = node.child_by_field_name("name") {
            return Some(self.text(name).to_owned());
        }
        // e.g. Go `type_declaration > type_spec(name)`
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find_map(|child| child.child_by_field_name("name"))
            .map(|name| self.text(name).to_owned())
    }

    fn signature(&self, node: Node<'_>) -> Option<String> {
        let start = node.start_byte();
        let end = node.child_by_field_name("body").map_or_else(
            || self.source[start..node.end_byte()].find('\n').map_or(node.end_byte(), |i| start + i),
            |body| body.start_byte(),
        );
        let raw = self.source.get(start..end)?;
        let mut signature = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let trimmed_len = signature.trim_end_matches(['{', ':', ' ']).len();
        signature.truncate(trimmed_len);
        if signature.len() > 240 {
            let cut = (0..=240).rev().find(|&i| signature.is_char_boundary(i)).unwrap_or(0);
            signature.truncate(cut);
            signature.push('…');
        }
        (!signature.is_empty()).then_some(signature)
    }
}

fn merge(acc: Option<Span>, next: Span) -> Span {
    match acc {
        Some(span) => Span { start: span.start, end: next.end, end_row: next.end_row },
        None => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(language: AstLanguage, source: &str, max_lines: usize) -> Vec<(ChunkKind, Option<String>, String)> {
        chunk(language, source, ChunkOptions { max_lines })
            .unwrap()
            .into_iter()
            .map(|p| (p.kind, p.symbol, source[p.start_byte..p.end_byte].to_owned()))
            .collect()
    }

    const RUST_SRC: &str = r#"use std::fmt;

/// Adds numbers.
#[inline]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub struct Point {
    x: i32,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.x)
    }

    fn other(&self) {}
}
"#;

    #[test]
    fn rust_definitions_are_atomic_with_docs() {
        let chunks = run(AstLanguage::Rust, RUST_SRC, 120);
        let kinds: Vec<_> = chunks.iter().map(|c| (c.0, c.1.as_deref())).collect();
        assert_eq!(
            kinds,
            vec![
                (ChunkKind::Preamble, None),
                (ChunkKind::Function, Some("add")),
                (ChunkKind::Struct, Some("Point")),
                (ChunkKind::Impl, Some("fmt::Display for Point")),
            ]
        );
        assert!(chunks[1].2.starts_with("/// Adds numbers.\n#[inline]\npub fn add"));
        assert!(chunks[1].2.ends_with('}'));
    }

    #[test]
    fn big_containers_are_split_into_members() {
        let pieces = chunk(AstLanguage::Rust, RUST_SRC, ChunkOptions { max_lines: 4 }).unwrap();
        let methods: Vec<_> = pieces
            .iter()
            .filter(|p| p.kind == ChunkKind::Method)
            .map(|p| (p.symbol.as_deref().unwrap(), p.parent.as_deref().unwrap()))
            .collect();
        assert_eq!(methods, vec![("fmt", "fmt::Display for Point"), ("other", "fmt::Display for Point")]);
        let header = pieces.iter().find(|p| p.kind == ChunkKind::Impl).unwrap();
        assert_eq!(header.signature.as_deref(), Some("impl fmt::Display for Point"));
    }

    #[test]
    fn python_decorators_and_methods() {
        let src = "import os\n\n@dataclass\nclass User:\n    name: str\n\n    def greet(self):\n        return 'hi'\n";
        let chunks = run(AstLanguage::Python, src, 120);
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[1].0, chunks[1].1.as_deref()), (ChunkKind::Class, Some("User")));
        assert!(chunks[1].2.starts_with("@dataclass"));

        let split = chunk(AstLanguage::Python, src, ChunkOptions { max_lines: 3 }).unwrap();
        assert!(split.iter().any(|p| p.kind == ChunkKind::Method && p.symbol.as_deref() == Some("greet")));
    }

    #[test]
    fn typescript_exports_and_interfaces() {
        let src = "export interface Props { name: string }\nexport function render(p: Props) {\n  return p.name;\n}\n";
        let chunks = run(AstLanguage::TypeScript, src, 120);
        let kinds: Vec<_> = chunks.iter().map(|c| (c.0, c.1.as_deref())).collect();
        assert_eq!(kinds, vec![(ChunkKind::Interface, Some("Props")), (ChunkKind::Function, Some("render"))]);
        assert!(chunks[1].2.starts_with("export function"));
    }

    #[test]
    fn go_methods_and_types() {
        let src = "package main\n\ntype Server struct{}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n";
        let chunks = run(AstLanguage::Go, src, 120);
        let kinds: Vec<_> = chunks.iter().map(|c| (c.0, c.1.as_deref())).collect();
        assert_eq!(
            kinds,
            vec![(ChunkKind::Preamble, None), (ChunkKind::Type, Some("Server")), (ChunkKind::Method, Some("Start"))]
        );
    }

    #[test]
    fn long_functions_are_split_but_keep_metadata() {
        let body = "    x += 1;\n".repeat(30);
        let src = format!("fn long() {{\n{body}}}\n");
        let pieces = chunk(AstLanguage::Rust, &src, ChunkOptions { max_lines: 10 }).unwrap();
        assert!(pieces.len() >= 3);
        assert!(pieces.iter().all(|p| p.symbol.as_deref() == Some("long")));
        assert!(pieces.iter().all(|p| p.signature.as_deref() == Some("fn long()")));
    }
}
