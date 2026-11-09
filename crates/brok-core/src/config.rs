//! `brok.yml`: the workspace configuration.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CoreError, Result};

/// The commented template written by `brok init`.
pub const DEFAULT_TEMPLATE: &str = include_str!("../templates/brok.yml");

/// Current configuration schema version.
pub const CONFIG_VERSION: u32 = 1;

/// Root of `brok.yml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrokConfig {
    /// Schema version.
    pub version: u32,
    /// Workspace topology.
    pub workspace: WorkspaceConfig,
    /// What gets indexed and how.
    pub index: IndexConfig,
    /// Local embedding model.
    pub embeddings: EmbeddingsConfig,
    /// Search tuning.
    pub search: SearchConfig,
    /// Memories kept between sessions.
    pub memory: MemoryConfig,
    /// Local usage metrics (`brok stats`).
    pub metrics: MetricsConfig,
    /// LLM routing (OpenAI-compatible providers).
    pub llm: LlmConfig,
    /// Index mirroring.
    pub sync: SyncConfig,
    /// Per-shard switches and settings, keyed by shard name.
    pub shards: BTreeMap<String, ShardEntry>,
}

impl Default for BrokConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            workspace: WorkspaceConfig::default(),
            index: IndexConfig::default(),
            embeddings: EmbeddingsConfig::default(),
            search: SearchConfig::default(),
            memory: MemoryConfig::default(),
            metrics: MetricsConfig::default(),
            llm: LlmConfig::default(),
            sync: SyncConfig::default(),
            shards: BTreeMap::new(),
        }
    }
}

/// `workspace:` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Index git submodules and nested git repositories as hot partitions.
    pub submodules: bool,
    /// Other workspaces this one depends on, keyed by name: their code and
    /// memories are searched too.
    pub links: BTreeMap<String, LinkConfig>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self { submodules: true, links: BTreeMap::new() }
    }
}

/// `workspace.links.<name>`: a workspace on disk (`path`) or a git
/// repository brok clones (`git`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinkConfig {
    /// Directory of the workspace, absolute or relative to this workspace's root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Git URL, cloned into `.brokspace/links/<name>/src`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// Branch of `git` (the remote's default branch when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What it is and how this workspace uses it (given to the LLM).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Search its memories too (its shared ones, and its private ones when it
    /// is a Brokspace workspace on this machine).
    #[serde(skip_serializing_if = "is_true")]
    pub memory: bool,
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self { path: None, git: None, branch: None, description: None, memory: true }
    }
}

impl LinkConfig {
    /// A link to a directory.
    pub fn local(path: impl Into<String>) -> Self {
        Self { path: Some(path.into()), ..Self::default() }
    }

    /// A link to a git repository.
    pub fn git(url: impl Into<String>, branch: Option<String>) -> Self {
        Self { git: Some(url.into()), branch, ..Self::default() }
    }
}

/// `metrics:` section. Metrics stay on this machine, in `.brokspace/metrics/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsConfig {
    /// Record timings and counts of searches, answers, indexing and errors.
    pub enabled: bool,
    /// Days of metrics kept; older months are deleted.
    pub retention_days: u32,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self { enabled: true, retention_days: 90 }
    }
}

/// `memory:` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    /// Search memories and let tools save them.
    pub enabled: bool,
    /// Save IDE chat sessions as memories.
    pub sessions: bool,
    /// Include the user's global memories, shared by every workspace.
    pub global: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { enabled: true, sessions: true, global: true }
    }
}

/// `index:` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IndexConfig {
    /// Include globs.
    pub include: Vec<String>,
    /// Exclude globs.
    pub exclude: Vec<String>,
    /// Maximum file size in KiB.
    pub max_file_kb: u64,
    /// Maximum lines per chunk before an AST node is split.
    pub max_chunk_lines: usize,
    /// Chunk store backend.
    pub store: StoreBackend,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            include: vec!["**/*".into()],
            exclude: [
                "**/.git/**",
                "**/.brokspace/**",
                "**/target/**",
                "**/node_modules/**",
                "**/dist/**",
                "**/build/**",
                "**/*.lock",
                "**/pnpm-lock.yaml",
                "**/package-lock.json",
                "**/npm-shrinkwrap.json",
                "**/go.sum",
                "**/*.min.js",
            ]
            .map(String::from)
            .to_vec(),
            max_file_kb: 512,
            max_chunk_lines: 120,
            store: StoreBackend::Auto,
        }
    }
}

/// Chunk store implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreBackend {
    /// LanceDB when compiled in, Arrow IPC otherwise.
    #[default]
    Auto,
    /// LanceDB (ANN vector index, scales to large workspaces).
    Lancedb,
    /// Memory-mapped Arrow IPC files with exact (flat) vector search.
    Arrow,
}

/// `embeddings:` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddingsConfig {
    /// Turn semantic search on or off.
    pub enabled: bool,
    /// Model id.
    pub model: String,
    /// Weights variant.
    pub variant: ModelVariant,
    /// Output dimensions (Matryoshka truncation).
    pub dimensions: usize,
    /// Execution device.
    pub device: Device,
    /// Texts per inference batch.
    pub batch_size: usize,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "nomic-embed-text-v1.5".into(),
            variant: ModelVariant::Quantized,
            dimensions: 768,
            device: Device::Auto,
            batch_size: 16,
        }
    }
}

/// Model weights variant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelVariant {
    /// int8 quantized weights.
    #[default]
    Quantized,
    /// fp32 weights.
    Full,
}

/// Inference device.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// Best available accelerator, falling back to CPU.
    #[default]
    Auto,
    /// CPU only.
    Cpu,
    /// NVIDIA CUDA.
    Cuda,
    /// Apple CoreML.
    Coreml,
    /// Windows DirectML.
    Directml,
}

/// `search:` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchConfig {
    /// Default number of hits.
    pub limit: usize,
    /// Weight applied to partitions other than the focused (hot) one.
    pub cold_weight: f32,
    /// Weight applied to results from linked workspaces.
    pub link_weight: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self { limit: 8, cold_weight: 0.85, link_weight: 0.8 }
    }
}

/// `llm:` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmConfig {
    /// Provider used when none is requested.
    pub default: String,
    /// Providers keyed by name.
    pub providers: BTreeMap<String, LlmProvider>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        let ollama = LlmProvider {
            base_url: "http://localhost:11434/v1".into(),
            model: "qwen2.5-coder:7b".into(),
            api_key_env: None,
            temperature: None,
            max_tokens: None,
        };
        Self { default: "ollama".into(), providers: BTreeMap::from([("ollama".into(), ollama)]) }
    }
}

/// An OpenAI-compatible chat completions endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmProvider {
    /// Base URL, up to and including `/v1`.
    pub base_url: String,
    /// Model name.
    pub model: String,
    /// Environment variable holding the API key (never the key itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Completion token limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// `sync:` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    /// Where the index is mirrored.
    pub backend: SyncBackend,
    /// Object storage settings (S3, R2, MinIO).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteStore>,
}

/// Index mirroring backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncBackend {
    /// Local only.
    #[default]
    None,
    /// AWS S3.
    S3,
    /// Cloudflare R2.
    R2,
    /// MinIO or any S3-compatible server.
    Minio,
    /// Commit the index through Git LFS.
    GitLfs,
}

impl SyncBackend {
    /// Whether the backend stores data in an object store.
    pub fn is_object_store(self) -> bool {
        matches!(self, Self::S3 | Self::R2 | Self::Minio)
    }
}

/// Object storage coordinates. Credentials are read from environment
/// variables at runtime and never stored in the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteStore {
    /// Bucket name.
    pub bucket: String,
    /// Key prefix inside the bucket.
    #[serde(default = "default_prefix")]
    pub prefix: String,
    /// Region (S3) or `auto` (R2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Custom endpoint (R2, MinIO).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Variable holding the access key id.
    #[serde(default = "default_access_key_env")]
    pub access_key_env: String,
    /// Variable holding the secret access key.
    #[serde(default = "default_secret_key_env")]
    pub secret_key_env: String,
}

fn default_prefix() -> String {
    "brokspace".into()
}
fn default_access_key_env() -> String {
    "AWS_ACCESS_KEY_ID".into()
}
fn default_secret_key_env() -> String {
    "AWS_SECRET_ACCESS_KEY".into()
}

/// `shards.<name>` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShardEntry {
    /// Whether `brok run` starts the shard.
    pub enabled: bool,
    /// Free-form settings handed to the shard on connect.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub settings: Value,
}

impl Default for ShardEntry {
    fn default() -> Self {
        Self { enabled: true, settings: Value::Null }
    }
}

impl BrokConfig {
    /// Parses YAML text and validates it.
    pub fn from_yaml(text: &str) -> Result<Self> {
        let config: Self = serde_yaml_ng::from_str(text).map_err(|e| CoreError::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Loads and validates a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
        Self::from_yaml(&text).map_err(|e| match e {
            CoreError::Config(msg) => CoreError::Config(format!("{}: {msg}", path.display())),
            other => other,
        })
    }

    /// Writes the config as YAML while preserving the existing file's comments,
    /// formatting, and key order.
    pub fn save(&self, path: &Path) -> Result<()> {
        let yaml = serde_yaml_ng::to_string(self).map_err(|e| CoreError::Config(e.to_string()))?;
        let existing = match std::fs::read_to_string(path) {
            Ok(existing) => existing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DEFAULT_TEMPLATE.to_owned(),
            Err(error) => return Err(CoreError::io(path, error)),
        };
        let merged = merge_yaml(&existing, &yaml);
        crate::fsutil::write_atomic(path, merged.as_bytes())
    }

    /// Semantic checks that serde cannot express.
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();
        if self.version != CONFIG_VERSION {
            errors.push(format!("unsupported version {} (expected {CONFIG_VERSION})", self.version));
        }
        if !self.llm.providers.is_empty() && !self.llm.providers.contains_key(&self.llm.default) {
            errors.push(format!("llm.default `{}` is not listed in llm.providers", self.llm.default));
        }
        if ![64, 128, 256, 512, 768].contains(&self.embeddings.dimensions) {
            errors.push("embeddings.dimensions must be one of 64, 128, 256, 512, 768".into());
        }
        if self.embeddings.batch_size == 0 {
            errors.push("embeddings.batch_size must be greater than zero".into());
        }
        if self.index.max_chunk_lines < 10 {
            errors.push("index.max_chunk_lines must be at least 10".into());
        }
        if !(0.0..=1.0).contains(&self.search.cold_weight) {
            errors.push("search.cold_weight must be between 0 and 1".into());
        }
        if !(0.0..=1.0).contains(&self.search.link_weight) {
            errors.push("search.link_weight must be between 0 and 1".into());
        }
        if !(1..=3650).contains(&self.metrics.retention_days) {
            errors.push("metrics.retention_days must be between 1 and 3650".into());
        }
        for (name, link) in &self.workspace.links {
            if !is_link_name(name) {
                errors.push(format!(
                    "workspace.links: `{name}` is not a valid name (letters, digits, `-`, `_` and `.`, not starting with `.`)"
                ));
            }
            match (&link.path, &link.git) {
                (Some(_), Some(_)) => errors.push(format!("workspace.links.{name}: set `path` or `git`, not both")),
                (None, None) => errors.push(format!("workspace.links.{name}: `path` or `git` is required")),
                (Some(_), None) if link.branch.is_some() => {
                    errors.push(format!("workspace.links.{name}: `branch` only applies to `git` links"));
                }
                _ => {}
            }
        }
        if self.sync.backend.is_object_store() && self.sync.remote.is_none() {
            errors.push("sync.remote is required for object storage backends (run `brok setup`)".into());
        }
        if errors.is_empty() { Ok(()) } else { Err(CoreError::Config(errors.join("; "))) }
    }

    /// Whether a shard is enabled (shards absent from the file are enabled).
    pub fn shard_enabled(&self, name: &str) -> bool {
        self.shards.get(name).is_none_or(|entry| entry.enabled)
    }

    /// Settings of a shard (`null` when absent).
    pub fn shard_settings(&self, name: &str) -> Value {
        self.shards.get(name).map(|entry| entry.settings.clone()).unwrap_or(Value::Null)
    }
}

/// Whether `name` can name a link (it becomes a directory name).
pub fn is_link_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

struct KeyLine {
    path: String,
    index: usize,
    indent: usize,
    has_children: bool,
}

fn merge_yaml(existing: &str, serialized: &str) -> String {
    let desired_lines: Vec<String> = serialized.lines().map(str::to_owned).collect();
    let mut lines: Vec<String> = existing.lines().map(str::to_owned).collect();
    let desired_keys = key_lines(&desired_lines);

    for desired in desired_keys {
        let current_keys = key_lines(&lines);
        if let Some(current) = current_keys.iter().find(|key| key.path == desired.path) {
            if desired.has_children {
                if !current.has_children {
                    lines[current.index] = replace_yaml_value(&lines[current.index], "");
                }
            } else if !current.has_children {
                lines[current.index] =
                    replace_yaml_value(&lines[current.index], &desired_lines[desired.index]);
            }
            continue;
        }

        let block = yaml_block(&desired_lines, desired.index, desired.indent);
        let parent_path = desired.path.rsplit_once('.').map(|(parent, _)| parent);
        let insertion = if let Some(parent_path) = parent_path {
            let Some(parent) = current_keys.iter().find(|key| key.path == parent_path) else {
                continue;
            };
            if !parent.has_children {
                lines[parent.index] = replace_yaml_value(&lines[parent.index], "");
            }
            let refreshed = key_lines(&lines);
            let Some(parent) = refreshed.iter().find(|key| key.path == parent_path) else {
                continue;
            };
            insertion_index(&lines, parent)
        } else {
            let mut index = lines.len();
            if index > 0 && !lines[index - 1].trim().is_empty() {
                lines.insert(index, String::new());
                index += 1;
            }
            index
        };
        lines.splice(insertion..insertion, block);
    }

    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn key_lines(lines: &[String]) -> Vec<KeyLine> {
    let mut locator = Locator::default();
    let mut keys = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let Some(Located::Key(path)) = locator.locate(line) else {
            continue;
        };
        let indent = line.len() - line.trim_start().len();
        let has_children = lines[index + 1..].iter().find_map(|next| {
            let trimmed = next.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let next_indent = next.len() - trimmed.len();
            Some(next_indent > indent || (trimmed.starts_with('-') && next_indent >= indent))
        }).unwrap_or(false);
        keys.push(KeyLine { path, index, indent, has_children });
    }
    keys
}

fn yaml_block(lines: &[String], start: usize, indent: usize) -> Vec<String> {
    let mut end = start + 1;
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line_indent = line.len() - trimmed.len();
        if line_indent <= indent && !(trimmed.starts_with('-') && line_indent >= indent) {
            break;
        }
        end = index + 1;
    }
    lines[start..end].to_vec()
}

fn insertion_index(lines: &[String], parent: &KeyLine) -> usize {
    let mut end = parent.index + 1;
    for (index, line) in lines.iter().enumerate().skip(parent.index + 1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            let indent = line.len() - trimmed.len();
            if indent > parent.indent {
                end = index + 1;
            }
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent <= parent.indent && !(trimmed.starts_with('-') && indent >= parent.indent) {
            break;
        }
        end = index + 1;
    }
    end
}

fn replace_yaml_value(existing: &str, desired: &str) -> String {
    let Some(colon) = existing.find(':') else {
        return desired.to_owned();
    };
    let desired_value = desired
        .find(':')
        .map(|desired_colon| desired[desired_colon + 1..].trim())
        .unwrap_or(desired);
    let after_colon = &existing[colon + 1..];
    let spacing = after_colon.chars().take_while(|character| character.is_whitespace()).count();
    let comment = inline_comment_start(existing).map(|comment_index| {
        let mut start = comment_index;
        while start > colon + 1 + spacing
            && existing.as_bytes()[start - 1].is_ascii_whitespace()
        {
            start -= 1;
        }
        &existing[start..]
    }).unwrap_or("");

    format!(
        "{}{}{}{}",
        &existing[..=colon],
        &after_colon[..spacing],
        desired_value,
        comment
    )
}

fn inline_comment_start(line: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('\"') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
        } else if character == '\"' || character == '\'' {
            quote = Some(character);
        } else if character == '#'
            && index > 0
            && line[..index].chars().next_back().is_some_and(char::is_whitespace)
        {
            return Some(index);
        }
    }
    None
}

enum Located {
    Key(String),
    Item,
}

#[derive(Default)]
struct Locator {
    stack: Vec<(usize, String)>,
}

impl Locator {
    fn locate(&mut self, line: &str) -> Option<Located> {
        let content = line.trim_start();
        let indent = line.len() - content.len();
        let content = content.trim_end();
        if content.is_empty() || content.starts_with('#') {
            return None;
        }
        let unquote = |value: &str| value.trim().trim_matches(['\"', '\'']).to_owned();
        if content.starts_with('-') {
            while self.stack.last().is_some_and(|(level, _)| *level > indent) {
                self.stack.pop();
            }
            return Some(Located::Item);
        }
        let (key, _) = content.split_once(':')?;
        while self.stack.last().is_some_and(|(level, _)| *level >= indent) {
            self.stack.pop();
        }
        self.stack.push((indent, unquote(key)));
        Some(Located::Key(self.path()))
    }

    fn path(&self) -> String {
        self.stack.iter().map(|(_, key)| key.as_str()).collect::<Vec<_>>().join(".")
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_preserves_existing_comments_and_key_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brok.yml");
        let original = DEFAULT_TEMPLATE
            .replace("version: 1\n", "version: 1\n# Keep this custom header comment.\n")
            .replace(
                "      model: qwen2.5-coder:7b\n",
                "      # Keep this custom provider comment.\n      model: qwen2.5-coder:7b\n",
            )
            .replace(
                "# Mirror the index to shared storage.",
                "# Keep this custom sync comment.\n# Mirror the index to shared storage.",
            );
        std::fs::write(&path, &original).unwrap();

        let mut config = BrokConfig::from_yaml(&original).unwrap();
        config.llm.default = "lmstudio".into();
        config.llm.providers.get_mut("ollama").unwrap().model = "qwen2.5-coder:1.5b".into();
        config.embeddings.enabled = false;
        config.sync.backend = SyncBackend::GitLfs;
        config.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        for comment in original.lines().filter(|line| line.trim_start().starts_with('#')) {
            assert!(text.contains(comment), "comment `{comment}` was lost:\n{text}");
        }
        assert!(text.contains("  default: lmstudio"));
        assert!(text.contains("      model: qwen2.5-coder:1.5b"));
        assert!(text.contains("  enabled: false"));
        assert!(text.contains("  backend: git-lfs"));

        let sections = ["workspace:", "index:", "embeddings:", "search:", "memory:", "metrics:", "llm:", "sync:", "shards:"];
        let positions: Vec<_> = sections.iter().map(|section| text.find(section).unwrap()).collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert_eq!(BrokConfig::load(&path).unwrap(), config);
    }

    #[test]
    fn save_inserts_new_keys_into_their_existing_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brok.yml");
        let mut config = BrokConfig::from_yaml(DEFAULT_TEMPLATE).unwrap();
        config.llm.providers.get_mut("ollama").unwrap().temperature = Some(0.25);
        config.sync.backend = SyncBackend::S3;
        config.sync.remote = Some(RemoteStore {
            bucket: "brok-index".into(),
            prefix: "shared".into(),
            region: Some("us-east-1".into()),
            endpoint: None,
            access_key_env: "AWS_ACCESS_KEY_ID".into(),
            secret_key_env: "AWS_SECRET_ACCESS_KEY".into(),
        });
        config.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let ollama = text.find("    ollama:").unwrap();
        let model = text.find("      model: qwen2.5-coder:7b").unwrap();
        let temperature = text.find("      temperature: 0.25").unwrap();
        let next_provider = text.find("    lmstudio:").unwrap();
        assert!(ollama < model && model < temperature && temperature < next_provider, "{text}");

        let sync = text.find("sync:\n").unwrap();
        let backend = text.find("  backend: s3").unwrap();
        let remote = text.find("  remote:\n").unwrap();
        let bucket = text.find("    bucket: brok-index").unwrap();
        assert!(sync < backend && backend < remote && remote < bucket, "{text}");
        assert_eq!(BrokConfig::load(&path).unwrap(), config);
    }

    #[test]
    fn template_is_valid_and_matches_defaults() {
        let config = BrokConfig::from_yaml(DEFAULT_TEMPLATE).unwrap();
        let defaults = BrokConfig::default();
        assert_eq!(config.index, defaults.index);
        assert_eq!(config.embeddings, defaults.embeddings);
        assert_eq!(config.search, defaults.search);
        assert_eq!(config.llm.default, defaults.llm.default);
        assert_eq!(config.llm.providers["ollama"], defaults.llm.providers["ollama"]);
        assert!(config.shard_enabled("brok-agent"));
        assert_eq!(config.shard_settings("brok-agent")["watch_interval_secs"], 5);
    }

    #[test]
    fn empty_file_uses_defaults() {
        assert_eq!(BrokConfig::from_yaml("{}").unwrap(), BrokConfig::default());
    }

    #[test]
    fn typos_are_reported() {
        let err = BrokConfig::from_yaml("embedings:\n  enabled: false\n").unwrap_err();
        assert!(err.to_string().contains("embedings"), "{err}");
    }

    #[test]
    fn semantic_validation() {
        let err = BrokConfig::from_yaml("llm:\n  default: nope\n").unwrap_err().to_string();
        assert!(err.contains("llm.default"), "{err}");
        let err = BrokConfig::from_yaml("sync:\n  backend: s3\n").unwrap_err().to_string();
        assert!(err.contains("sync.remote"), "{err}");
    }

    #[test]
    fn links() {
        let config = BrokConfig::from_yaml(
            "workspace:\n  links:\n    checkout:\n      path: ../checkout\n    payments:\n      git: https://example.com/p.git\n      branch: main\n      memory: false\n",
        )
        .unwrap();
        assert_eq!(config.workspace.links["checkout"], LinkConfig::local("../checkout"));
        assert!(!config.workspace.links["payments"].memory);

        let err = BrokConfig::from_yaml("workspace:\n  links:\n    x:\n      path: a\n      git: b\n").unwrap_err();
        assert!(err.to_string().contains("not both"), "{err}");
        let err = BrokConfig::from_yaml("workspace:\n  links:\n    x: {}\n").unwrap_err();
        assert!(err.to_string().contains("is required"), "{err}");
        let err = BrokConfig::from_yaml("workspace:\n  links:\n    ../x:\n      path: a\n").unwrap_err();
        assert!(err.to_string().contains("valid name"), "{err}");
    }

    #[test]
    fn roundtrip_through_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brok.yml");
        let mut config = BrokConfig::from_yaml(DEFAULT_TEMPLATE).unwrap();
        config.sync.backend = SyncBackend::GitLfs;
        config.save(&path).unwrap();
        assert_eq!(BrokConfig::load(&path).unwrap(), config);
    }
}
