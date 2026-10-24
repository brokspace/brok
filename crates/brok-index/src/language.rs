//! Language detection by file extension.

use std::path::Path;

/// Languages with a dedicated syntax-aware chunker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AstLanguage {
    /// Rust
    Rust,
    /// Python
    Python,
    /// JavaScript (incl. JSX)
    JavaScript,
    /// TypeScript
    TypeScript,
    /// TSX
    Tsx,
    /// Go
    Go,
    /// Java
    Java,
}

/// What we know about a file's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Language {
    /// Short identifier stored with every chunk (`rust`, `markdown`, `yaml`, ...).
    pub id: &'static str,
    /// Syntax-aware chunker, if any.
    pub ast: Option<AstLanguage>,
    /// Markdown-style section chunking.
    pub markdown: bool,
}

impl Language {
    const fn ast(id: &'static str, lang: AstLanguage) -> Self {
        Self { id, ast: Some(lang), markdown: false }
    }

    const fn text(id: &'static str) -> Self {
        Self { id, ast: None, markdown: false }
    }

    /// Detects the language of `path` from its extension or file name.
    pub fn detect(path: &Path) -> Self {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        match file_name {
            "Dockerfile" | "Containerfile" => return Self::text("dockerfile"),
            "Makefile" | "GNUmakefile" => return Self::text("make"),
            _ => {}
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Self::ast("rust", AstLanguage::Rust),
            "py" | "pyi" => Self::ast("python", AstLanguage::Python),
            "js" | "jsx" | "mjs" | "cjs" => Self::ast("javascript", AstLanguage::JavaScript),
            "ts" | "mts" | "cts" => Self::ast("typescript", AstLanguage::TypeScript),
            "tsx" => Self::ast("tsx", AstLanguage::Tsx),
            "go" => Self::ast("go", AstLanguage::Go),
            "java" => Self::ast("java", AstLanguage::Java),
            "md" | "mdx" | "markdown" => Self { id: "markdown", ast: None, markdown: true },
            "c" | "h" => Self::text("c"),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => Self::text("cpp"),
            "cs" => Self::text("csharp"),
            "kt" | "kts" => Self::text("kotlin"),
            "swift" => Self::text("swift"),
            "scala" => Self::text("scala"),
            "rb" => Self::text("ruby"),
            "php" => Self::text("php"),
            "sh" | "bash" | "zsh" => Self::text("shell"),
            "ps1" | "psm1" => Self::text("powershell"),
            "sql" => Self::text("sql"),
            "yml" | "yaml" => Self::text("yaml"),
            "toml" => Self::text("toml"),
            "json" | "jsonc" => Self::text("json"),
            "xml" => Self::text("xml"),
            "html" | "htm" => Self::text("html"),
            "css" | "scss" | "less" => Self::text("css"),
            "proto" => Self::text("protobuf"),
            "tf" | "hcl" => Self::text("hcl"),
            "vue" => Self::text("vue"),
            "svelte" => Self::text("svelte"),
            _ => Self::text("text"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_languages() {
        assert_eq!(Language::detect(Path::new("src/main.rs")).ast, Some(AstLanguage::Rust));
        assert_eq!(Language::detect(Path::new("a/b.TSX")).ast, Some(AstLanguage::Tsx));
        assert!(Language::detect(Path::new("README.md")).markdown);
        assert_eq!(Language::detect(Path::new("Dockerfile")).id, "dockerfile");
        assert_eq!(Language::detect(Path::new("notes")).id, "text");
    }
}
