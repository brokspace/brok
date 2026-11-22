//! Synthetic workspaces, generated from a fixed seed.
//!
//! The same size always produces byte-identical files, so two runs (or two
//! machines) benchmark the same input. The files look like code in five
//! languages plus Markdown docs, with identifiers drawn from a small
//! vocabulary, so BM25 has realistic term overlap. Nothing is downloaded.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// How big the workspace is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// About 50 files: a service or a library.
    Small,
    /// About 1,500 files: a typical application.
    Medium,
    /// About 15,000 files: a large monorepo.
    Large,
}

impl Size {
    /// Lowercase name, as used on the command line and in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// Parses `small`, `medium` or `large`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            other => bail!("unknown size `{other}` (small, medium or large)"),
        }
    }

    /// Source files in the root repository.
    pub fn files(self) -> usize {
        match self {
            Self::Small => 50,
            Self::Medium => 1_500,
            Self::Large => 15_000,
        }
    }
}

/// What was generated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixture {
    /// Workspace root.
    pub root: PathBuf,
    /// Source files, all repositories together.
    pub files: usize,
    /// Their size in bytes.
    pub bytes: u64,
    /// Git repositories, the root included.
    pub repositories: usize,
    /// Queries that match the generated content, for the search scenario.
    pub queries: Vec<String>,
}

const NOUNS: [&str; 24] = [
    "invoice",
    "payment",
    "customer",
    "order",
    "cart",
    "session",
    "token",
    "profile",
    "ledger",
    "refund",
    "shipment",
    "catalog",
    "inventory",
    "tenant",
    "webhook",
    "report",
    "account",
    "coupon",
    "address",
    "receipt",
    "subscription",
    "notification",
    "audit",
    "schedule",
];
const VERBS: [&str; 16] = [
    "create",
    "update",
    "validate",
    "refresh",
    "cancel",
    "export",
    "import",
    "reconcile",
    "archive",
    "verify",
    "retry",
    "dispatch",
    "resolve",
    "merge",
    "render",
    "compute",
];

/// Deterministic xorshift64* generator: no dependency, stable across platforms.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Generates the workspace under `root` (which must be empty or missing),
/// with `nested` extra independent repositories and one submodule when
/// `nested > 0`.
pub fn generate(root: &Path, size: Size, nested: usize) -> Result<Fixture> {
    std::fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut files = 0;
    let mut bytes = 0;

    let (f, b) = write_sources(root, size.files(), &mut rng, "app")?;
    files += f;
    bytes += b;
    std::fs::write(root.join(".gitignore"), "target/\nnode_modules/\n*.log\n")?;
    git_init(root)?;
    let mut repositories = 1;

    for n in 0..nested {
        let dir = root.join("services").join(format!("service-{n}"));
        let (f, b) = write_sources(&dir, (size.files() / 10).max(10), &mut rng, &format!("svc{n}"))?;
        files += f;
        bytes += b;
        git_init(&dir)?;
        repositories += 1;
    }
    if nested > 0 {
        // A submodule cloned from a local repository: no network.
        let origin = tempfile::tempdir().context("temp dir for the submodule origin")?;
        let (f, b) = write_sources(origin.path(), (size.files() / 10).max(10), &mut rng, "shared")?;
        files += f;
        bytes += b;
        git_init(origin.path())?;
        git(
            root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                &origin.path().to_string_lossy(),
                "libs/shared",
            ],
        )?;
        git(root, &["add", "-A"])?;
        git(root, &["commit", "-q", "-m", "add the shared library"])?;
        repositories += 1;
    }

    let queries = (0..20).map(|_| format!("{} {} {}", rng.pick(&VERBS), rng.pick(&NOUNS), rng.pick(&NOUNS))).collect();
    Ok(Fixture { root: root.to_path_buf(), files, bytes, repositories, queries })
}

/// Rewrites one source file, for the incremental scenario.
pub fn touch_one_file(root: &Path) -> Result<()> {
    let path = root.join("src").join("module_0").join("file_0.rs");
    let mut text = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    text.push_str("\npub fn bench_incremental_marker() -> u32 {\n    42\n}\n");
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))
}

fn write_sources(dir: &Path, count: usize, rng: &mut Rng, prefix: &str) -> Result<(usize, u64)> {
    let mut bytes = 0;
    for i in 0..count {
        let module = i % 40;
        let (ext, body) = match i % 5 {
            0 => ("rs", rust_file(rng, prefix, i)),
            1 => ("py", python_file(rng, prefix, i)),
            2 => ("ts", typescript_file(rng, prefix, i)),
            3 => ("go", go_file(rng, prefix, i)),
            _ => ("java", java_file(rng, prefix, i)),
        };
        let path = dir.join("src").join(format!("module_{module}")).join(format!("file_{}.{ext}", i / 40));
        std::fs::create_dir_all(path.parent().unwrap_or(dir))?;
        bytes += body.len() as u64;
        std::fs::write(&path, body)?;
        if i % 25 == 0 {
            let doc = doc_file(rng, prefix, i);
            bytes += doc.len() as u64;
            std::fs::write(dir.join("src").join(format!("module_{module}")).join(format!("notes_{}.md", i / 40)), doc)?;
        }
    }
    Ok((count, bytes))
}

fn ident(rng: &mut Rng, i: usize) -> (String, String, String) {
    let (verb, noun) = (rng.pick(&VERBS).to_owned(), rng.pick(&NOUNS).to_owned());
    (verb.clone(), noun.clone(), format!("{verb}_{noun}_{i}"))
}

fn rust_file(rng: &mut Rng, prefix: &str, i: usize) -> String {
    let mut s = format!("//! {prefix}: module {i}.\n\nuse std::collections::HashMap;\n\n");
    for n in 0..(3 + rng.below(4)) {
        let (verb, noun, name) = ident(rng, i * 10 + n as usize);
        let _ = write!(
            s,
            "/// {verb} the {noun} and report what changed.\npub fn {name}(input: &HashMap<String, String>) -> Result<usize, String> {{\n    let mut changed = 0;\n    for (key, value) in input {{\n        if key.contains(\"{noun}\") && !value.is_empty() {{\n            changed += value.len();\n        }}\n    }}\n    if changed == 0 {{\n        return Err(format!(\"nothing to {verb} for {noun}\"));\n    }}\n    Ok(changed)\n}}\n\n"
        );
    }
    s
}

fn python_file(rng: &mut Rng, prefix: &str, i: usize) -> String {
    let mut s = format!("\"\"\"{prefix}: module {i}.\"\"\"\n\nfrom dataclasses import dataclass\n\n\n");
    for n in 0..(3 + rng.below(4)) {
        let (verb, noun, name) = ident(rng, i * 10 + n as usize);
        let _ = write!(
            s,
            "@dataclass\nclass {class}{n}:\n    {noun}_id: int\n    amount: float = 0.0\n\n    def {name}(self, retries: int = 3) -> bool:\n        \"\"\"{verb} the {noun}; give up after `retries` attempts.\"\"\"\n        for attempt in range(retries):\n            if self.amount > attempt:\n                return True\n        return False\n\n\n",
            class = noun.chars().next().map_or_else(String::new, |c| c.to_uppercase().collect::<String>()) + &noun[1..],
        );
    }
    s
}

fn typescript_file(rng: &mut Rng, prefix: &str, i: usize) -> String {
    let mut s =
        format!("// {prefix}: module {i}.\nexport interface Options {{ retries: number; dryRun: boolean }}\n\n");
    for n in 0..(3 + rng.below(4)) {
        let (verb, noun, name) = ident(rng, i * 10 + n as usize);
        let _ = write!(
            s,
            "/** {verb} the {noun}. */\nexport async function {name}(id: string, options: Options): Promise<boolean> {{\n  for (let attempt = 0; attempt < options.retries; attempt++) {{\n    if (!options.dryRun && id.startsWith(\"{noun}\")) {{\n      return true;\n    }}\n  }}\n  return false;\n}}\n\n"
        );
    }
    s
}

fn go_file(rng: &mut Rng, prefix: &str, i: usize) -> String {
    let mut s =
        format!("// Package {prefix}{i} holds generated benchmark code.\npackage {prefix}{i}\n\nimport \"errors\"\n\n");
    for n in 0..(3 + rng.below(4)) {
        let (verb, noun, name) = ident(rng, i * 10 + n as usize);
        let _ = write!(
            s,
            "// {name} will {verb} the {noun}.\nfunc {name}(id int, limit int) (int, error) {{\n\tif id > limit {{\n\t\treturn 0, errors.New(\"{noun} out of range\")\n\t}}\n\treturn id * 2, nil\n}}\n\n"
        );
    }
    s
}

fn java_file(rng: &mut Rng, prefix: &str, i: usize) -> String {
    let mut s =
        format!("package com.example.{prefix};\n\n/** Generated benchmark class {i}. */\npublic class Service{i} {{\n");
    for n in 0..(3 + rng.below(4)) {
        let (verb, noun, name) = ident(rng, i * 10 + n as usize);
        let _ = write!(
            s,
            "    /** {verb} the {noun}. */\n    public boolean {name}(int id, int limit) {{\n        if (id > limit) {{\n            throw new IllegalArgumentException(\"{noun} out of range\");\n        }}\n        return id % 2 == 0;\n    }}\n\n"
        );
    }
    s.push_str("}\n");
    s
}

fn doc_file(rng: &mut Rng, prefix: &str, i: usize) -> String {
    let (verb, noun) = (rng.pick(&VERBS), rng.pick(&NOUNS));
    format!(
        "# {prefix} notes {i}\n\nHow we {verb} a {noun}: check the {noun} id, retry twice, then report the failure.\n\n## Operations\n\nThe {noun} service owns this flow. See the runbook before you {verb} it by hand.\n"
    )
}

fn git_init(dir: &Path) -> Result<()> {
    git(dir, &["init", "-q", "-b", "main"])?;
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-q", "-m", "initial commit"])
}

/// Runs the user's real `git` with a fixed identity, outside any shim.
fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=bench", "-c", "user.email=bench@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .context("run git (is it installed?)")?;
    if !out.status.success() {
        bail!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push((
                        path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"),
                        std::fs::read(&path).unwrap(),
                    ));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn same_seed_same_bytes() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let fa = generate(a.path(), Size::Small, 0).unwrap();
        let fb = generate(b.path(), Size::Small, 0).unwrap();
        assert_eq!(tree(a.path()), tree(b.path()));
        assert_eq!((fa.files, fa.bytes, &fa.queries), (fb.files, fb.bytes, &fb.queries));
        assert_eq!(fa.files, 50);
        assert!(fa.bytes > 10_000);
        assert!(a.path().join(".git").is_dir());
    }

    #[test]
    fn nested_repositories_and_a_submodule() {
        let tmp = tempfile::tempdir().unwrap();
        let f = generate(tmp.path(), Size::Small, 2).unwrap();
        assert_eq!(f.repositories, 4);
        assert!(tmp.path().join("services/service-1/.git").exists());
        assert!(tmp.path().join("libs/shared/.git").exists());
        assert!(tmp.path().join(".gitmodules").is_file());
    }

    #[test]
    fn queries_use_the_vocabulary() {
        let tmp = tempfile::tempdir().unwrap();
        let f = generate(tmp.path(), Size::Small, 0).unwrap();
        assert_eq!(f.queries.len(), 20);
        for q in &f.queries {
            let words: Vec<&str> = q.split(' ').collect();
            assert!(VERBS.contains(&words[0]) && NOUNS.contains(&words[1]) && NOUNS.contains(&words[2]), "{q}");
        }
    }

    #[test]
    fn touching_a_file_changes_it() {
        let tmp = tempfile::tempdir().unwrap();
        generate(tmp.path(), Size::Small, 0).unwrap();
        let path = tmp.path().join("src/module_0/file_0.rs");
        let before = std::fs::read(&path).unwrap();
        touch_one_file(tmp.path()).unwrap();
        assert!(std::fs::read(&path).unwrap().len() > before.len());
    }

    #[test]
    fn sizes_parse() {
        assert_eq!(Size::parse("medium").unwrap().files(), 1_500);
        assert!(Size::parse("huge").is_err());
    }
}
