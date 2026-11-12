//! Sentence embeddings with TaylorAI/bge-micro-v2 (01 §8).
use crate::error::{Error, Kind, Result};
use crate::text::strip;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::path::PathBuf;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

pub const MODEL: &str = "TaylorAI/bge-micro-v2";
const BATCH: usize = 32;

/// Anything that turns texts into vectors; tests substitute fakes.
pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f64>>>;
}

pub struct Bge {
    model: BertModel,
    tok: Tokenizer,
}

fn hub_dir() -> PathBuf {
    use crate::config::var;
    if let Some(c) = var("HF_HUB_CACHE") {
        return PathBuf::from(c);
    }
    if let Some(h) = var("HF_HOME") {
        return PathBuf::from(h).join("hub");
    }
    crate::config::home().join(".cache").join("huggingface").join("hub")
}

impl Bge {
    /// Loads the model from the Hugging Face cache snapshot named by refs/main.
    pub fn load() -> std::result::Result<Bge, String> {
        let repo = hub_dir().join("models--TaylorAI--bge-micro-v2");
        let rev = std::fs::read_to_string(repo.join("refs").join("main")).map_err(|e| format!("{e}: {}", repo.display()))?;
        let snap = repo.join("snapshots").join(rev.trim());
        let cfg: Config = std::fs::read(snap.join("config.json"))
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))?;
        let mut tok = Tokenizer::from_file(snap.join("tokenizer.json")).map_err(|e| e.to_string())?;
        tok.with_truncation(Some(TruncationParams { max_length: 512, ..Default::default() })).map_err(|e| e.to_string())?;
        tok.with_padding(Some(PaddingParams { strategy: PaddingStrategy::BatchLongest, ..Default::default() }));
        let weights = std::fs::read(snap.join("model.safetensors")).map_err(|e| e.to_string())?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu).map_err(|e| e.to_string())?;
        let model = BertModel::load(vb, &cfg).map_err(|e| e.to_string())?;
        Ok(Bge { model, tok })
    }

    fn batch(&self, texts: &[&str]) -> candle_core::Result<Vec<Vec<f64>>> {
        let enc = self.tok.encode_batch(texts.iter().map(|t| strip(t).to_string()).collect::<Vec<_>>(), true).map_err(candle_core::Error::msg)?;
        let ids: Vec<Tensor> = enc.iter().map(|e| Tensor::new(e.get_ids(), &Device::Cpu)).collect::<candle_core::Result<_>>()?;
        let mask: Vec<Tensor> =
            enc.iter().map(|e| Tensor::new(e.get_attention_mask(), &Device::Cpu)).collect::<candle_core::Result<_>>()?;
        let ids = Tensor::stack(&ids, 0)?;
        let mask = Tensor::stack(&mask, 0)?;
        let hidden = self.model.forward(&ids, &ids.zeros_like()?, Some(&mask))?;
        let m = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let sum = hidden.broadcast_mul(&m)?.sum(1)?;
        let count = m.sum(1)?.clamp(1e-9, f64::MAX)?;
        let pooled = sum.broadcast_div(&count)?;
        Ok(pooled.to_vec2::<f32>()?.into_iter().map(|v| v.into_iter().map(f64::from).collect()).collect())
    }
}

impl Embedder for Bge {
    /// Batches texts of similar length (less padding), then restores the input order.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f64>>> {
        let mut order: Vec<usize> = (0..texts.len()).collect();
        order.sort_by_key(|&i| texts[i].len());
        let mut out = vec![Vec::new(); texts.len()];
        for idx in order.chunks(BATCH) {
            let chunk: Vec<&str> = idx.iter().map(|&i| texts[i]).collect();
            let vecs = self.batch(&chunk).map_err(|e| Error::new(Kind::Internal, format!("embedding failed: {e}")))?;
            for (&i, v) in idx.iter().zip(vecs) {
                out[i] = v;
            }
        }
        Ok(out)
    }
}
