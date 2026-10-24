//! nomic-embed-text-v1.5 on ONNX Runtime.

use std::borrow::Cow;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use brok_core::config::{Device, EmbeddingsConfig};
use ort::environment::Environment;
use ort::ep::ExecutionProviderDispatch;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use super::{EmbedKind, Embedder, ModelFiles, postprocess};

/// Longest input in tokens. The embedding text starts with the path, symbol
/// and signature, which carry most of a chunk's meaning; the tail of very
/// long chunks is still covered by BM25. Inference cost grows at least
/// linearly (attention quadratically) with this value.
const MAX_TOKENS: usize = 512;

pub(super) struct NomicEmbedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    input_names: Vec<String>,
    dims: usize,
    batch_size: usize,
    // The environment must outlive every session created from it.
    _environment: Environment,
}

impl NomicEmbedder {
    pub(super) fn load(files: &ModelFiles, config: &EmbeddingsConfig) -> Result<Self> {
        let environment = ort::init().with_name("brok").build().context("initializing ONNX Runtime")?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        let mut builder = Session::builder(&environment)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(ort::Error::<()>::from)?
            .with_intra_threads(threads)
            .map_err(ort::Error::<()>::from)?
            .with_execution_providers(execution_providers(config.device))
            .map_err(ort::Error::<()>::from)?;
        let session =
            builder.commit_from_file(&files.model).with_context(|| format!("loading {}", files.model.display()))?;
        let input_names = session.inputs().iter().map(|input| input.name().to_owned()).collect();

        let mut tokenizer = Tokenizer::from_file(&files.tokenizer).map_err(|e| anyhow!("loading tokenizer: {e}"))?;
        tokenizer.with_padding(Some(PaddingParams { strategy: PaddingStrategy::BatchLongest, ..Default::default() }));
        tokenizer
            .with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() }))
            .map_err(|e| anyhow!("configuring tokenizer: {e}"))?;

        tracing::info!(model = %files.model.display(), dims = config.dimensions, "embedding model loaded");
        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            input_names,
            dims: config.dimensions,
            batch_size: config.batch_size.max(1),
            _environment: environment,
        })
    }

    fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let encodings = self.tokenizer.encode_batch(texts, true).map_err(|e| anyhow!("tokenizing: {e}"))?;
        let batch = encodings.len();
        let seq_len = encodings.first().map_or(0, |e| e.get_ids().len());
        if batch == 0 || seq_len == 0 {
            return Ok(vec![vec![0.0; self.dims]; batch]);
        }

        let mut ids = Vec::with_capacity(batch * seq_len);
        let mut mask = Vec::with_capacity(batch * seq_len);
        for encoding in &encodings {
            ids.extend(encoding.get_ids().iter().map(|&id| i64::from(id)));
            mask.extend(encoding.get_attention_mask().iter().map(|&m| i64::from(m)));
        }
        let shape = [batch, seq_len];
        let mut inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = Vec::with_capacity(self.input_names.len());
        for name in &self.input_names {
            let data = match name.as_str() {
                "input_ids" => ids.clone(),
                "attention_mask" => mask.clone(),
                "token_type_ids" => vec![0i64; batch * seq_len],
                other => return Err(anyhow!("unexpected model input `{other}`")),
            };
            inputs.push((Cow::Owned(name.clone()), Tensor::from_array((shape, data))?.into()));
        }

        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let outputs = session.run(inputs)?;
        let (out_shape, hidden) = outputs[0].try_extract_tensor::<f32>()?;
        let hidden_dim = usize::try_from(*out_shape.last().context("empty output shape")?)?;

        Ok((0..batch)
            .map(|b| {
                let start = b * seq_len * hidden_dim;
                let rows = &hidden[start..start + seq_len * hidden_dim];
                postprocess(rows, &mask[b * seq_len..(b + 1) * seq_len], seq_len, hidden_dim, self.dims)
            })
            .collect())
    }
}

impl Embedder for NomicEmbedder {
    fn dims(&self) -> usize {
        self.dims
    }

    fn model_id(&self) -> &str {
        "nomic-embed-text-v1.5"
    }

    fn embed(&self, texts: &[String], kind: EmbedKind) -> Result<Vec<Vec<f32>>> {
        let prefix = match kind {
            EmbedKind::Document => "search_document: ",
            EmbedKind::Query => "search_query: ",
        };
        // Batches are padded to their longest text: grouping texts of similar
        // length avoids paying for padding (attention cost is quadratic).
        let mut order: Vec<usize> = (0..texts.len()).collect();
        order.sort_by_key(|&i| texts[i].len());
        let mut vectors: Vec<Vec<f32>> = vec![Vec::new(); texts.len()];
        for batch in order.chunks(self.batch_size) {
            let prefixed = batch.iter().map(|&i| format!("{prefix}{}", texts[i])).collect();
            for (&i, vector) in batch.iter().zip(self.embed_batch(prefixed)?) {
                vectors[i] = vector;
            }
        }
        Ok(vectors)
    }
}

/// Execution providers to try, in order; ONNX Runtime falls back to the CPU
/// when none of them is available on the machine.
fn execution_providers(device: Device) -> Vec<ExecutionProviderDispatch> {
    #[allow(unused_mut)] // only mutated when a GPU feature is enabled
    let mut providers = Vec::new();
    let wants = |d: Device| device == Device::Auto || device == d;
    #[cfg(feature = "cuda")]
    if wants(Device::Cuda) {
        providers.push(ort::ep::CUDA::default().build());
    }
    #[cfg(feature = "coreml")]
    if wants(Device::Coreml) {
        providers.push(ort::ep::CoreML::default().build());
    }
    #[cfg(feature = "directml")]
    if wants(Device::Directml) {
        providers.push(ort::ep::DirectML::default().build());
    }
    if device != Device::Auto && device != Device::Cpu && providers.is_empty() {
        tracing::warn!(?device, "this build has no support for the requested device; using the CPU");
    }
    let _ = wants;
    providers
}
