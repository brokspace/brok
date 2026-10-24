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
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self { submodules: true }
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
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self { limit: 8, cold_weight: 0.85 }
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

    /// Writes the config as YAML (comments of the original file are not kept).
    pub fn save(&self, path: &Path) -> Result<()> {
        let yaml = serde_yaml_ng::to_string(self).map_err(|e| CoreError::Config(e.to_string()))?;
        let text = format!(
            "# Brokspace workspace configuration.\n\
             # Reference: https://github.com/brokspace/brok/blob/main/docs/configuration.md\n{yaml}"
        );
        crate::fsutil::write_atomic(path, text.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn roundtrip_through_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brok.yml");
        let mut config = BrokConfig::from_yaml(DEFAULT_TEMPLATE).unwrap();
        config.sync.backend = SyncBackend::GitLfs;
        config.save(&path).unwrap();
        assert_eq!(BrokConfig::load(&path).unwrap(), config);
    }
}
