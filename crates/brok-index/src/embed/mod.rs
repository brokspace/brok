//! Local text embeddings.
//!
//! The default model is `nomic-embed-text-v1.5`, executed in-process with
//! ONNX Runtime ([`nomic`], feature `onnx`). Weights are downloaded once
//! into `.brokspace/cache/models/` (or `$BROK_MODELS_DIR` to share them
//! between workspaces).

#[cfg(feature = "onnx")]
mod nomic;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use brok_core::config::{EmbeddingsConfig, ModelVariant};
use futures::StreamExt;
use tokio::io::AsyncWriteExt;

/// Environment variable overriding the model cache directory.
pub use brok_core::user::MODELS_DIR_ENV;

const NOMIC_ID: &str = "nomic-embed-text-v1.5";
const NOMIC_BASE_URL: &str = "https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main";

/// What a text is used for; nomic models are trained with task prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    /// Indexed content.
    Document,
    /// Search query.
    Query,
}

/// A text embedding model.
pub trait Embedder: Send + Sync {
    /// Output dimensions.
    fn dims(&self) -> usize;
    /// Model identifier.
    fn model_id(&self) -> &str;
    /// Embeds texts into L2-normalized vectors. Blocking (CPU/GPU bound):
    /// call it from `spawn_blocking` in async code.
    fn embed(&self, texts: &[String], kind: EmbedKind) -> Result<Vec<Vec<f32>>>;
}

/// Local files of a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    /// Model directory.
    pub dir: PathBuf,
    /// ONNX graph.
    pub model: PathBuf,
    /// HuggingFace `tokenizer.json`.
    pub tokenizer: PathBuf,
    remote: Vec<(String, PathBuf)>,
}

impl ModelFiles {
    /// Where the configured model lives: a copy already inside the workspace
    /// (older installs) wins, otherwise the per-user cache shared by every
    /// workspace ([`MODELS_DIR_ENV`] or the OS cache directory).
    pub fn locate(workspace_models_dir: &Path, config: &EmbeddingsConfig) -> Result<Self> {
        if config.model != NOMIC_ID {
            bail!("unsupported embedding model `{}` (supported: {NOMIC_ID})", config.model);
        }
        let local = workspace_models_dir.join(NOMIC_ID);
        let dir = if std::env::var_os(MODELS_DIR_ENV).is_none() && local.join("tokenizer.json").is_file() {
            local
        } else {
            brok_core::user::models_dir().unwrap_or_else(|| workspace_models_dir.to_path_buf()).join(NOMIC_ID)
        };
        let onnx_name = match config.variant {
            ModelVariant::Quantized => "model_quantized.onnx",
            ModelVariant::Full => "model.onnx",
        };
        let model = dir.join(onnx_name);
        let tokenizer = dir.join("tokenizer.json");
        let remote = vec![
            (format!("{NOMIC_BASE_URL}/tokenizer.json"), tokenizer.clone()),
            (format!("{NOMIC_BASE_URL}/onnx/{onnx_name}"), model.clone()),
        ];
        Ok(Self { dir, model, tokenizer, remote })
    }

    /// Whether every file is present.
    pub fn is_present(&self) -> bool {
        self.model.is_file() && self.tokenizer.is_file()
    }

    /// Downloads missing files. `progress(file, downloaded, total)` is
    /// called as bytes arrive.
    pub async fn download(&self, progress: &(dyn Fn(&str, u64, Option<u64>) + Send + Sync)) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await.with_context(|| format!("creating {}", self.dir.display()))?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("brok/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(20))
            .build()?;
        for (url, path) in &self.remote {
            if path.is_file() {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let response = client.get(url).send().await?.error_for_status().with_context(|| format!("GET {url}"))?;
            let total = response.content_length();
            let partial = path.with_extension("part");
            let mut file = tokio::fs::File::create(&partial).await?;
            let mut stream = response.bytes_stream();
            let mut downloaded = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.with_context(|| format!("downloading {name}"))?;
                file.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;
                progress(&name, downloaded, total);
            }
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&partial, path).await?;
        }
        Ok(())
    }
}

/// Loads the configured embedder. Returns `Ok(None)` when embeddings are
/// disabled, and an error when they are enabled but cannot be loaded.
pub fn load_embedder(config: &EmbeddingsConfig, workspace_models_dir: &Path) -> Result<Option<Arc<dyn Embedder>>> {
    if !config.enabled {
        return Ok(None);
    }
    let files = ModelFiles::locate(workspace_models_dir, config)?;
    if !files.is_present() {
        bail!(
            "embedding model not found in {} (run `brok model download`, or set embeddings.enabled: false)",
            files.dir.display()
        );
    }
    #[cfg(feature = "onnx")]
    {
        Ok(Some(Arc::new(nomic::NomicEmbedder::load(&files, config)?)))
    }
    #[cfg(not(feature = "onnx"))]
    {
        bail!("this brok build has no ONNX support (feature `onnx`); set embeddings.enabled: false")
    }
}

/// Mean pooling + layer norm + Matryoshka truncation + L2 normalization,
/// as prescribed for nomic-embed-text-v1.5.
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub(crate) fn postprocess(hidden: &[f32], mask: &[i64], seq_len: usize, hidden_dim: usize, dims: usize) -> Vec<f32> {
    let mut pooled = vec![0f32; hidden_dim];
    let mut count = 0f32;
    for (token, &m) in mask.iter().enumerate().take(seq_len) {
        if m == 0 {
            continue;
        }
        count += 1.0;
        let row = &hidden[token * hidden_dim..(token + 1) * hidden_dim];
        pooled.iter_mut().zip(row).for_each(|(acc, v)| *acc += v);
    }
    let count = count.max(1.0);
    pooled.iter_mut().for_each(|v| *v /= count);

    let mean = pooled.iter().sum::<f32>() / hidden_dim as f32;
    let var = pooled.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / hidden_dim as f32;
    let std = (var + 1e-5).sqrt();
    let mut out: Vec<f32> = pooled.iter().take(dims).map(|v| (v - mean) / std).collect();

    let norm = out.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
    out.iter_mut().for_each(|v| *v /= norm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postprocess_pools_masks_and_normalizes() {
        // 2 tokens x 4 dims, second token masked out.
        let hidden = [1.0, 2.0, 3.0, 4.0, 100.0, 100.0, 100.0, 100.0];
        let v = postprocess(&hidden, &[1, 0], 2, 4, 2);
        assert_eq!(v.len(), 2);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
        assert!(v[0] < 0.0 && v[1] < 0.0, "below-mean dims are negative after layer norm: {v:?}");
    }

    #[test]
    fn locate_rejects_unknown_models() {
        let config = EmbeddingsConfig { model: "other".into(), ..EmbeddingsConfig::default() };
        assert!(ModelFiles::locate(Path::new("/m"), &config).is_err());
        let files = ModelFiles::locate(Path::new("/m"), &EmbeddingsConfig::default()).unwrap();
        assert!(files.model.ends_with("model_quantized.onnx"));
    }
}
