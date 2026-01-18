use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use std::fs::File;
use std::collections::HashMap;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;

use candle_core::{Device, Tensor, DType};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::llama as llama_model;
use candle_transformers::models::quantized_llama;
use crate::models::reasoner::{ReasonerModel, Config as ReasonerConfig};
use tokenizers::Tokenizer;

// Truly global lock to protect hardware across all instances
lazy_static! {
    static ref GLOBAL_HW_LOCK: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

#[async_trait]
pub trait LLMProvider: Send + Sync {
    async fn generate(&self, model: &str, prompt: String, system: Option<String>) -> Result<String>;
    async fn generate_stream(&self, model: &str, prompt: String, system: Option<String>) -> Result<BoxStream<'static, Result<String>>>;
    /// Get a clone of the hardware lock
    fn get_lock(&self) -> Arc<Mutex<()>>;
    /// Send a notification message back to the user/UI
    async fn notify(&self, _message: &str) -> Result<()> {
        Ok(())
    }
}

/// Provider that wraps another provider and publishes tokens/notifications to a broadcast channel
pub struct PublishingProvider {
    inner: Arc<dyn LLMProvider>,
    tx: tokio::sync::broadcast::Sender<String>,
}

impl PublishingProvider {
    pub fn new(inner: Arc<dyn LLMProvider>, tx: tokio::sync::broadcast::Sender<String>) -> Self {
        Self { inner, tx }
    }
}

#[async_trait]
impl LLMProvider for PublishingProvider {
    async fn generate(&self, model: &str, prompt: String, system: Option<String>) -> Result<String> {
        let mut stream = self.generate_stream(model, prompt, system).await?;
        let mut full_text = String::new();
        while let Some(chunk) = stream.next().await {
            full_text.push_str(&chunk?);
        }
        Ok(full_text)
    }

    async fn generate_stream(&self, model: &str, prompt: String, system: Option<String>) -> Result<BoxStream<'static, Result<String>>> {
        let stream = self.inner.generate_stream(model, prompt, system).await?;
        let tx = self.tx.clone();

        let mapped_stream = futures_util::stream::unfold((stream, String::new(), false), move |(mut s, mut buffer, mut answer_detected)| {
            let tx = tx.clone();
            async move {
                match s.next().await {
                    Some(Ok(token)) => {
                        buffer.push_str(&token);
                        
                        // Detect FPF transition from internal thought to external answer
                        if !answer_detected {
                            let upper_buffer = buffer.to_uppercase();
                            if upper_buffer.contains("[ANSWER]") || upper_buffer.contains("ANSWER:") || upper_buffer.contains("### ANSWER") {
                                answer_detected = true;
                                let _ = tx.send("STATE:ANSWER_START".to_string());
                            }
                        }
                        
                        let _ = tx.send(format!("TOKEN:{}", token));
                        Some((Ok(token), (s, buffer, answer_detected)))
                    }
                    Some(Err(e)) => Some((Err(e), (s, buffer, answer_detected))),
                    None => None,
                }
            }
        });

        Ok(Box::pin(mapped_stream))
    }

    fn get_lock(&self) -> Arc<Mutex<()>> {
        self.inner.get_lock()
    }

    async fn notify(&self, message: &str) -> Result<()> {
        let _ = self.tx.send(message.to_string());
        self.inner.notify(message).await
    }
}

enum LoadedModel {
    Llama(llama_model::Llama, Arc<Mutex<llama_model::Cache>>, Tokenizer),
    Quantized(Arc<Mutex<quantized_llama::ModelWeights>>, Tokenizer),
    Reasoner(Arc<Mutex<ReasonerModel>>, Tokenizer),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfig {
    name: String,
    repo: String,
    revision: String,
    tokenizer_repo: String,
    is_quantized: bool,
    quant_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    models: Vec<ModelConfig>,
    defaults: HashMap<String, String>,
}

pub struct CandleProvider {
    device: Device,
    models: Arc<Mutex<HashMap<String, LoadedModel>>>, 
    lock: Arc<Mutex<()>>,
}

impl CandleProvider {
    pub fn new() -> Result<Self> {
        let device = if candle_core::utils::metal_is_available() {
            let force_cpu = std::env::var("FORCE_CPU").ok().map(|v| v == "1").unwrap_or(false)
                || std::env::var("AGENCY_FORCE_CPU").ok().map(|v| v == "1").unwrap_or(false);
            
            if force_cpu {
                println!("⚠️  Forcing CPU device via environment variable.");
                Device::Cpu
            } else {
                Device::new_metal(0).unwrap_or(Device::Cpu)
            }
        } else {
            Device::Cpu
        };

        Ok(Self {
            device,
            models: Arc::new(Mutex::new(HashMap::new())),
            lock: GLOBAL_HW_LOCK.clone(),
        })
    }

    async fn get_or_load_model(&self, model_name: &str) -> Result<()> {
        let mut models = self.models.lock().await;
        
        // Load registry from file
        let registry_path = "config/agency_models.json";
        let registry_file = File::open(registry_path).context("Failed to open agency_models.json")?;
        let registry: Registry = serde_json::from_reader(registry_file).context("Failed to parse agency_models.json")?;
        // Resolve alias if needed
        let resolved_name = if let Some(actual_name) = registry.defaults.get(model_name) {
            actual_name.as_str()
        } else {
            model_name
        };

        if models.contains_key(resolved_name) {
            return Ok(())
        }

        let config = registry.models.iter()
            .find(|m| m.name == resolved_name)
            .cloned()
            .unwrap_or_else(|| {
                // Default fallback if not in registry
                ModelConfig {
                    name: resolved_name.to_string(),
                    repo: "meta-llama/Llama-3.2-3B-Instruct".to_string(),
                    revision: "main".to_string(),
                    tokenizer_repo: "meta-llama/Llama-3.2-3B-Instruct".to_string(),
                    is_quantized: false,
                    quant_file: None,
                }
            });

        println!("🏠 Loading native model: {} on {:?}", config.name, self.device);

        let device = self.device.clone();
        let repo_id = config.repo.clone();
        let revision = config.revision.clone();
        let tokenizer_repo = config.tokenizer_repo.clone();
        let is_quantized = config.is_quantized;
        let quant_file = config.quant_file.clone();
        let hf_token = std::env::var("HF_TOKEN").ok();
        let model_name_owned = model_name.to_string();

        let loaded = tokio::task::spawn_blocking(move || -> Result<LoadedModel> {
            use hf_hub::{api::sync::ApiBuilder, Repo};
            let mut api_builder = ApiBuilder::new().with_progress(true);
            if let Some(token) = hf_token {
                api_builder = api_builder.with_token(Some(token));
            }
            let api = api_builder.build()?;
            
            let t_repo = api.repo(Repo::with_revision(
                tokenizer_repo,
                hf_hub::RepoType::Model,
                "main".to_string(),
            ));
            let tokenizer_filename = t_repo.get("tokenizer.json")?;
            let tokenizer = Tokenizer::from_file(tokenizer_filename).map_err(anyhow::Error::msg)?;

            let repo = api.repo(Repo::with_revision(
                repo_id.clone(),
                hf_hub::RepoType::Model,
                revision,
            ));

            let get_model_paths = |repo: &hf_hub::api::sync::ApiRepo| -> Result<Vec<std::path::PathBuf>> {
                if let Ok(path) = repo.get("model.safetensors") {
                    return Ok(vec![path]);
                }
                // Try sharded
                if let Ok(index_path) = repo.get("model.safetensors.index.json") {
                    let index_file = std::fs::File::open(index_path)?;
                    let index: serde_json::Value = serde_json::from_reader(index_file)?;
                    let weight_map = index["weight_map"]
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("Invalid index file: missing weight_map"))?;
                    
                    let mut shards = std::collections::HashSet::new();
                    for value in weight_map.values() {
                        if let Some(shard) = value.as_str() {
                            shards.insert(shard.to_string());
                        }
                    }
                    let mut shards: Vec<_> = shards.into_iter().collect();
                    shards.sort();
                    let mut paths = Vec::new();
                    for shard in shards {
                        paths.push(repo.get(&shard)?);
                    }
                    return Ok(paths);
                }
                Err(anyhow::anyhow!("Model weights not found (missing model.safetensors and index.json)"))
            };

            if is_quantized {
                let model_filename = quant_file.unwrap_or_else(|| "model.gguf".to_string());
                let model_path = repo.get(&model_filename)?;
                let mut file = std::fs::File::open(&model_path)?;
                let gguf_content = candle_core::quantized::gguf_file::Content::read(&mut file)?;
                let model = quantized_llama::ModelWeights::from_gguf(gguf_content, &mut file, &device)?;
                Ok(LoadedModel::Quantized(Arc::new(Mutex::new(model)), tokenizer))
            } else if repo_id.to_lowercase().contains("qwen") {
                let model_paths = get_model_paths(&repo)?;
                // Security: Validate model paths are safe before mmap
                for path in &model_paths {
                    if !path.exists() || path.to_str().map(|s| s.contains("..")).unwrap_or(true) {
                        return Err(anyhow::anyhow!("Security: Malicious model path detected: {:?}", path));
                    }
                }
                let vb = unsafe { candle_nn::VarBuilder::from_mmaped_safetensors(&model_paths, DType::F16, &device)? };
                
                let config = if repo_id.contains("1.5B") || model_name_owned.contains("1.5b") {
                    ReasonerConfig::qwen_1_5b()
                } else if repo_id.contains("0.5B") || model_name_owned.contains("0.5b") {
                    ReasonerConfig::qwen_0_5b()
                } else {
                    ReasonerConfig::qwen_7b()
                };

                let model = ReasonerModel::new(&config, vb)?;
                Ok(LoadedModel::Reasoner(Arc::new(Mutex::new(model)), tokenizer))
            } else {
                let model_paths = get_model_paths(&repo)?;
                // Security: Validate model paths
                for path in &model_paths {
                    if !path.exists() || path.to_str().map(|s| s.contains("..")).unwrap_or(true) {
                        return Err(anyhow::anyhow!("Security: Malicious model path detected: {:?}", path));
                    }
                }
                let vb = unsafe { candle_nn::VarBuilder::from_mmaped_safetensors(&model_paths, DType::F16, &device)? };
                
                let config_filename = repo.get("config.json")?;
                let config_json: serde_json::Value = serde_json::from_reader(File::open(config_filename)?)?;
                
                let config = llama_model::Config {
                    hidden_size: config_json["hidden_size"].as_u64().unwrap_or(2048) as usize,
                    intermediate_size: config_json["intermediate_size"].as_u64().unwrap_or(5632) as usize,
                    num_attention_heads: config_json["num_attention_heads"].as_u64().unwrap_or(32) as usize,
                    num_hidden_layers: config_json["num_hidden_layers"].as_u64().unwrap_or(22) as usize,
                    num_key_value_heads: config_json["num_key_value_heads"].as_u64().unwrap_or(4) as usize,
                    vocab_size: config_json["vocab_size"].as_u64().unwrap_or(32000) as usize,
                    rms_norm_eps: config_json["rms_norm_eps"].as_f64().unwrap_or(1e-5),
                    rope_theta: config_json["rope_theta"].as_f64().unwrap_or(10000.0) as f32,
                    bos_token_id: config_json["bos_token_id"].as_u64().map(|v| v as u32),
                    eos_token_id: Some(llama_model::LlamaEosToks::Single(config_json["eos_token_id"].as_u64().unwrap_or(2) as u32)),
                    tie_word_embeddings: config_json["tie_word_embeddings"].as_bool().unwrap_or(false),
                    use_flash_attn: false,
                    max_position_embeddings: config_json["max_position_embeddings"].as_u64().unwrap_or(4096) as usize,
                    rope_scaling: None,
                };

                let model = llama_model::Llama::load(vb, &config)?;
                let cache = llama_model::Cache::new(true, DType::F16, &config, &device)?;
                Ok(LoadedModel::Llama(model, Arc::new(Mutex::new(cache)), tokenizer))
            }
        }).await??;

        models.insert(model_name.to_string(), loaded);
        Ok(())
    }
}

#[async_trait]
impl LLMProvider for CandleProvider {
    async fn generate(&self, model_name: &str, prompt: String, system: Option<String>) -> Result<String> {
        let mut stream = self.generate_stream(model_name, prompt, system).await?;
        let mut full_text = String::new();
        while let Some(chunk) = stream.next().await {
            full_text.push_str(&chunk?);
        }
        Ok(full_text)
    }

    async fn generate_stream(&self, model_name: &str, prompt: String, system: Option<String>) -> Result<BoxStream<'static, Result<String>>> {
        let lock = self.lock.clone();
        let device = self.device.clone();
        
        self.get_or_load_model(model_name).await?;
        let mut models_guard = self.models.lock().await;
        let loaded_model = models_guard.get_mut(model_name).context("Model failed to load")?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        match loaded_model {
            LoadedModel::Llama(model, cache_lock, tokenizer) => {
                let model = model.clone();
                let cache_lock = cache_lock.clone();
                let tokenizer = tokenizer.clone();
                
                let model_name_lower = model_name.to_lowercase();
                
                // Smart Wrapping for Llama 3.x tags
                let has_bos = prompt.contains("<|begin_of_text|>");
                let is_already_formatted = prompt.contains("<|start_header_id|>") || has_bos;

                let full_prompt = if is_already_formatted {
                    prompt
                } else if let Some(sys) = system {
                    format!("<|start_header_id|>system<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n", sys, prompt)
                } else {
                    format!("<|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n", prompt)
                };

                let add_bos = !model_name_lower.contains("qwen") && !has_bos;
                let mut tokens = tokenizer.encode(full_prompt, add_bos).map_err(anyhow::Error::msg)?.get_ids().to_vec();
                let mut lp = LogitsProcessor::new(42, Some(0.7), Some(0.9));

                tokio::task::spawn_blocking(move || {
                    let mut cache = futures::executor::block_on(cache_lock.lock());
                    cache.clear();
                    
                    for step in 0..1024 {
                        let _guard = futures::executor::block_on(lock.lock());
                        let context_size = if step > 0 { 1 } else { tokens.len() };
                        let start_pos = tokens.len().saturating_sub(context_size);
                        let input_data = &tokens[start_pos..];
                        
                        let input = match Tensor::new(input_data, &device).and_then(|t| t.unsqueeze(0)).and_then(|t| t.to_dtype(DType::I64)) {
                            Ok(i) => i,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Input tensor error: {}", e))); break; }
                        };

                        let logits = match model.forward(&input, start_pos, &mut cache) {
                            Ok(l) => l,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Forward pass error: {}", e))); break; }
                        };

                        let logits = logits.squeeze(0).map_err(|e| anyhow::anyhow!("Squeeze error: {}", e)).unwrap_or(logits);
                        let logits_f32 = match logits.to_dtype(DType::F32) {
                            Ok(l) => l,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("DType conversion error: {}", e))); break; }
                        };

                        let next_token = match lp.sample(&logits_f32) {
                            Ok(t) => t,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Sampling error: {}", e))); break; }
                        };
                        
                        if next_token == 2 || next_token == 128001 || next_token == 128008 || next_token == 128009 || next_token == 151643 {
                            break;
                        }
                        
                        match tokenizer.decode(&[next_token], true) {
                            Ok(chunk) => {
                                tokens.push(next_token);
                                if tx.send(Ok(chunk)).is_err() { break; }
                            },
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Decode error: {}", e))); break; }
                        }
                    }
                });
            },
            LoadedModel::Quantized(model_mutex, tokenizer) => {
                let model_mutex = model_mutex.clone();
                let tokenizer = tokenizer.clone();
                
                let model_name_lower = model_name.to_lowercase();
                
                // Smart Wrapping: Check if prompt already contains special tokens
                let has_bos = if model_name_lower.contains("qwen") {
                    prompt.contains("<|im_start|>")
                } else if model_name_lower.contains("llama") {
                    prompt.contains("<|start_header_id|>") || prompt.contains("<|begin_of_text|>")
                } else {
                    prompt.contains("<|system|>") || prompt.contains("<|user|>")
                };

                let full_prompt = if has_bos {
                    prompt
                } else if model_name_lower.contains("qwen") {
                    if let Some(sys) = system {
                        format!("<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n", sys, prompt)
                    } else {
                        format!("<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n", prompt)
                    }
                } else if model_name_lower.contains("llama") {
                    if let Some(sys) = system {
                        format!("<|start_header_id|>system<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n", sys, prompt)
                    } else {
                        format!("<|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n", prompt)
                    }
                } else {
                    // Generic fallback
                    if let Some(sys) = system {
                        format!("<|system|>\n{}<|user|>\n{}<|assistant|>\n", sys, prompt)
                    }
                    else {
                        format!("<|user|>\n{}<|assistant|>\n", prompt)
                    }
                };

                let add_bos = !model_name_lower.contains("qwen") && !has_bos;
                let mut tokens = tokenizer.encode(full_prompt, add_bos).map_err(anyhow::Error::msg)?.get_ids().to_vec();
                let mut lp = LogitsProcessor::new(42, Some(0.7), Some(0.9));

                tokio::task::spawn_blocking(move || {
                    let mut model = futures::executor::block_on(model_mutex.lock());
                    
                    for step in 0..1024 {
                        let _guard = futures::executor::block_on(lock.lock());
                        let context_size = if step > 0 { 1 } else { tokens.len() };
                        let start_pos = tokens.len().saturating_sub(context_size);
                        
                        let input = match Tensor::new(&tokens[start_pos..], &device).and_then(|t| t.unsqueeze(0)).and_then(|t| t.to_dtype(DType::I64)) {
                            Ok(i) => i,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Input tensor error: {}", e))); break; }
                        };
                        
                        let logits = match model.forward(&input, start_pos) {
                            Ok(l) => l,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Forward pass error: {}", e))); break; }
                        };

                        let logits = logits.squeeze(0).map_err(|e| anyhow::anyhow!("Squeeze error: {}", e)).unwrap_or(logits);
                        let logits_f32 = match logits.to_dtype(DType::F32) {
                            Ok(l) => l,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("DType conversion error: {}", e))); break; }
                        };

                        let next_token = match lp.sample(&logits_f32) {
                            Ok(t) => t,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Sampling error: {}", e))); break; }
                        };
                        
                        // Enhanced EOS check for multiple model families
                        let is_eos = if model_name_lower.contains("qwen") {
                            next_token == 151643 || next_token == 151645
                        } else if model_name_lower.contains("llama") {
                            next_token == 128001 || next_token == 128008 || next_token == 128009
                        } else {
                            next_token == 2 // Standard Llama/generic EOS
                        };

                        if is_eos {
                            break;
                        }
                        
                        match tokenizer.decode(&[next_token], true) {
                            Ok(chunk) => {
                                tokens.push(next_token);
                                if tx.send(Ok(chunk)).is_err() { break; }
                            },
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Decode error: {}", e))); break; }
                        }
                    }
                });
            },
            LoadedModel::Reasoner(model_mutex, tokenizer) => {
                let model_mutex = model_mutex.clone();
                let tokenizer = tokenizer.clone();
                
                let full_prompt = if prompt.contains("<|im_start|>") {
                    prompt
                } else if let Some(sys) = system {
                    format!("<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n", sys, prompt)
                } else {
                    format!("<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n", prompt)
                };

                let add_bos = false; // Qwen models don't use BOS
                let mut tokens = tokenizer.encode(full_prompt, add_bos).map_err(anyhow::Error::msg)?.get_ids().to_vec();
                let mut lp = LogitsProcessor::new(42, Some(0.7), Some(0.9));

                tokio::task::spawn_blocking(move || {
                    let mut model = futures::executor::block_on(model_mutex.lock());
                    model.clear_cache();
                    
                    for step in 0..2048 {
                        let _guard = futures::executor::block_on(lock.lock());
                        let context_size = if step > 0 { 1 } else { tokens.len() };
                        let start_pos = tokens.len().saturating_sub(context_size);
                        
                        let input = match Tensor::new(&tokens[start_pos..], &device).and_then(|t| t.unsqueeze(0)).and_then(|t| t.to_dtype(DType::I64)) {
                            Ok(i) => i,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Input tensor error: {}", e))); break; }
                        };
                        
                        let logits = match model.forward(&input, start_pos) {
                            Ok(l) => l,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Forward pass error: {}", e))); break; }
                        };

                        let logits = logits.squeeze(0).map_err(|e| anyhow::anyhow!("Squeeze error: {}", e)).unwrap_or(logits);
                        let logits_f32 = match logits.to_dtype(DType::F32) {
                            Ok(l) => l,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("DType conversion error: {}", e))); break; }
                        };

                        let next_token = match lp.sample(&logits_f32) {
                            Ok(t) => t,
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Sampling error: {}", e))); break; }
                        };
                        
                        // Qwen EOS tokens
                        if next_token == 151643 || next_token == 151645 {
                            break;
                        }
                        
                        match tokenizer.decode(&[next_token], true) {
                            Ok(chunk) => {
                                tokens.push(next_token);
                                if tx.send(Ok(chunk)).is_err() { break; }
                            },
                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Decode error: {}", e))); break; }
                        }
                    }
                });
            }
        }
        
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {{
            rx.recv().await.map(|val| (val, rx))
        }});
        Ok(Box::pin(stream))
    }

    fn get_lock(&self) -> Arc<Mutex<()>> {
        self.lock.clone()
    }
}

pub struct OllamaProvider {
    client: ollama_rs::Ollama,
    lock: Arc<Mutex<()>>,
}

impl OllamaProvider {
    pub fn new(client: ollama_rs::Ollama) -> Self {
        Self {
            client,
            lock: GLOBAL_HW_LOCK.clone(),
        }
    }
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    async fn generate(&self, model: &str, prompt: String, system: Option<String>) -> Result<String> {
        let mut stream = self.generate_stream(model, prompt, system).await?;
        let mut full_text = String::new();
        while let Some(chunk) = stream.next().await {
            full_text.push_str(&chunk?);
        }
        Ok(full_text)
    }

    async fn generate_stream(&self, model: &str, prompt: String, system: Option<String>) -> Result<BoxStream<'static, Result<String>>> {
        use ollama_rs::generation::chat::{request::ChatMessageRequest, ChatMessage};
        use ollama_rs::models::ModelOptions;        
        let client = self.client.clone();
        let model = model.to_string();

        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(ChatMessage::system(sys));
        }
        messages.push(ChatMessage::user(prompt));

        let mut options = ModelOptions::default();
        options = options.num_ctx(4096);
        options = options.num_thread(4);

        let request = ChatMessageRequest::new(model, messages).options(options);

        let stream = client.send_chat_messages_stream(request).await?;

        let mapped_stream = stream.map(|res| {
            match res {
                Ok(chunk) => Ok(chunk.message.content),
                Err(e) => Err(anyhow::anyhow!("Ollama stream error: {:?}", e)),
            }
        });

        Ok(Box::pin(mapped_stream))
    }

    fn get_lock(&self) -> Arc<Mutex<()>> {
        self.lock.clone()
    }
}

pub struct OpenAICompatibleProvider {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    lock: Arc<Mutex<()>>,
}

pub struct RemoteNexusProvider {
    client: Client,
    url: String,
    lock: Arc<Mutex<()>>,
}

impl RemoteNexusProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            url: "http://localhost:8002/v1/chat/completions".to_string(),
            lock: GLOBAL_HW_LOCK.clone(),
        }
    }
}

#[async_trait]
impl LLMProvider for RemoteNexusProvider {
    async fn generate(&self, _model: &str, prompt: String, system: Option<String>) -> Result<String> {
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(json!({ "role": "system", "content": sys }));
        }
        messages.push(json!({ "role": "user", "content": prompt }));

        let body = json!({
            "messages": messages,
            "max_tokens": 1024,
        });

        let res = self.client.post(&self.url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // SOTA: The Nexus response is now strictly projected via MVPK logic.
        Ok(res["choices"][0]["message"]["content"].as_str().map(|s| s.to_string()).unwrap_or_else(|| "No response from Remote Nexus".to_string()))
    }

    async fn generate_stream(&self, _model: &str, prompt: String, system: Option<String>) -> Result<BoxStream<'static, Result<String>>> {
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(json!({ "role": "system", "content": sys }));
        }
        messages.push(json!({ "role": "user", "content": prompt }));

        let body = json!({
            "messages": messages,
            "max_tokens": 1024,
            "stream": true,
        });

        let res = self.client.post(&self.url)
            .json(&body)
            .send()
            .await?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::task::spawn(async move {
            let mut stream = res.bytes_stream();
            while let Some(item) = stream.next().await {
                match item {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        for line in text.lines() {
                            if line.starts_with("data: ") {
                                let data = &line[6..];
                                if data == "[DONE]" { break; }
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(content) = json["choices"][0]["delta"]["content"].as_str() {
                                        // SOTA: Forward raw tokens; higher-level agents will perform MVPK projection.
                                        let _ = tx.send(Ok(content.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("Remote stream error: {}", e)));
                        break;
                    }
                }
            }
        });

        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|val| (val, rx))
        });
        Ok(Box::pin(stream))
    }

    fn get_lock(&self) -> Arc<Mutex<()>> {
        self.lock.clone()
    }
}

impl OpenAICompatibleProvider {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url,
            api_key,
            lock: GLOBAL_HW_LOCK.clone(),
        }
    }
}

#[async_trait]
impl LLMProvider for OpenAICompatibleProvider {
    async fn generate(&self, model: &str, prompt: String, system: Option<String>) -> Result<String> {
        let mut stream = self.generate_stream(model, prompt, system).await?;
        let mut full_text = String::new();
        while let Some(chunk) = stream.next().await {
            full_text.push_str(&chunk?);
        }
        Ok(full_text)
    }

    async fn generate_stream(&self, model: &str, prompt: String, system: Option<String>) -> Result<BoxStream<'static, Result<String>>> {
        let mut messages = Vec::new();
        if let Some(sys) = system {
            messages.push(json!({ "role": "system", "content": sys }));
        }
        messages.push(json!({ "role": "user", "content": prompt }));

        let body = json!({
            "model": model,
            "messages": messages,
            "temperature": 0.7,
            "stream": true,
        });

        let mut request = self.client.post(format!("{}/chat/completions", self.base_url.trim_end_matches('/')))
            .json(&body);

        if let Some(ref key) = self.api_key {
            request = request.bearer_auth(key);
        }

        let res = request.send().await?.error_for_status()?;
        
        let stream = res.bytes_stream();
        let mapped_stream = stream.map(|res| {
            match res {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut content = String::new();
                    for line in text.lines() {
                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if data == "[DONE]" { continue; }
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                if let Some(chunk) = json["choices"][0]["delta"]["content"].as_str() {
                                    content.push_str(chunk);
                                }
                            }
                        }
                    }
                    Ok(content)
                }
                Err(e) => Err(anyhow::anyhow!("OpenAI stream error: {}", e)),
            }
        });

        Ok(Box::pin(mapped_stream))
    }

    fn get_lock(&self) -> Arc<Mutex<()>> {
        self.lock.clone()
    }
}

pub fn dynamic_provider() -> Arc<dyn LLMProvider> {
    let provider_type = std::env::var("AGENCY_PROVIDER").unwrap_or_else(|_| "nexus".to_string());
    
    match provider_type.to_lowercase().as_str() {
        "ollama" => {
            let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost".to_string());
            let port = std::env::var("OLLAMA_PORT").unwrap_or_else(|_| "11434".to_string())
                .parse::<u16>().unwrap_or(11434);
            
            println!("🦙 Initializing Ollama Provider at {}:{}...", host, port);
            let client = ollama_rs::Ollama::new(host, port);
            Arc::new(OllamaProvider::new(client))
        }
        "turbo" | "ollama-cloud" => {
            let base_url = "https://api.ollama.com/v1".to_string();
            let api_key = std::env::var("OLLAMA_API_KEY").ok();
            
            println!("🚀 Initializing Ollama Turbo Cloud (SOTA) at {}...", base_url);
            Arc::new(OpenAICompatibleProvider::new(base_url, api_key))
        }
        "openai" | "cloud" => {
            let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
            let api_key = std::env::var("OPENAI_API_KEY").ok();
            
            println!("☁️  Initializing OpenAI-Compatible Cloud Provider at {}...", base_url);
            Arc::new(OpenAICompatibleProvider::new(base_url, api_key))
        }
        "zai" | "glm" => {
            let base_url = "https://open.bigmodel.cn/api/paas/v4".to_string();
            let api_key = std::env::var("ZAI_API_KEY").ok();
            
            println!("🇨🇳 Initializing Z.ai (Zhipu AI) Provider at {}...", base_url);
            Arc::new(OpenAICompatibleProvider::new(base_url, api_key))
        }
        "candle" | "native" => {
            println!("🦀 Initializing Native Candle (Rust) Provider...");
            Arc::new(CandleProvider::new().expect("Failed to initialize Candle provider"))
        }
        _ => {
            println!("🌐 Initializing Remote Nexus Provider (Default)...");
            Arc::new(RemoteNexusProvider::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_candle_provider_qwen_tiny() -> Result<()> {
        if std::env::var("TEST_NATIVE").is_err() {
            println!("Skipping native inference test (TEST_NATIVE not set)");
            return Ok(())
        }

        let provider = CandleProvider::new()?;
        let response = provider.generate("qwen2.5-coder:0.5b", "Say 'Native Rust is active!'".to_string(), None).await?;
        
        println!("Native Model Response: {}", response);
        assert!(!response.is_empty());
        Ok(())
    }
}
