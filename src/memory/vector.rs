//! Vector Memory Implementation with Multi-Level Storage Tiering
//! 
//! Tiers:
//! 1. HOT: Active in RAM (RwLock<Vec>) - High speed, frequent access.
//! 2. COLD: Memory-Mapped (mmap) - Infinite lifespan, zero-RAM overhead until touched.
//! 3. COMPRESSED: Persisted Zstd on disk.

use anyhow::{Context, Result};
use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug, error};
use reqwest::Client;
use serde_json::json;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use rayon::prelude::*;
use memmap2::Mmap;

use super::{Memory, MemoryEntry};

pub enum VectorMemory {
    Local(LocalVectorMemory),
    Remote(RemoteVectorMemory),
}

impl VectorMemory {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let use_remote = std::env::var("AGENCY_USE_REMOTE_MEMORY").unwrap_or_else(|_| "0".to_string()) == "1";
        
        if use_remote {
            let host = std::env::var("AGENCY_MEMORY_HOST").unwrap_or_else(|_| "localhost".to_string());
            let port = std::env::var("AGENCY_MEMORY_PORT").unwrap_or_else(|_| "3001".to_string());
            let url = format!("http://{}:{}", host, port);
            info!("Initializing RemoteVectorMemory at {}", url);
            Ok(VectorMemory::Remote(RemoteVectorMemory::new(url)))
        } else {
            info!("Initializing LocalVectorMemory (Native + Tiered) at {:?}", path);
            Ok(VectorMemory::Local(LocalVectorMemory::new(path)?))
        }
    }
}

#[async_trait]
impl Memory for VectorMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<String> {
        match self {
            Self::Local(m) => m.store(entry).await,
            Self::Remote(m) => m.store(entry).await,
        }
    }

    async fn search(&self, query: &str, top_k: usize, context: Option<&str>, kind: Option<crate::orchestrator::Kind>) -> Result<Vec<MemoryEntry>> {
        match self {
            Self::Local(m) => m.search(query, top_k, context, kind).await,
            Self::Remote(m) => m.search(query, top_k, context, kind).await,
        }
    }

    async fn get_recent(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        match self {
            Self::Local(m) => m.get_recent(limit).await,
            Self::Remote(m) => m.get_recent(limit).await,
        }
    }

    async fn count(&self) -> Result<usize> {
        match self {
            Self::Local(m) => m.count().await,
            Self::Remote(m) => m.count().await,
        }
    }

    async fn persist(&self) -> Result<()> {
        match self {
            Self::Local(m) => m.persist().await,
            Self::Remote(m) => m.persist().await,
        }
    }

    async fn consolidate(&self) -> Result<usize> {
        match self {
            Self::Local(m) => m.consolidate().await,
            Self::Remote(m) => m.consolidate().await,
        }
    }

    async fn get_cold_memories(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        match self {
            Self::Local(m) => m.get_cold_memories(limit).await,
            Self::Remote(m) => m.get_cold_memories(limit).await,
        }
    }

    async fn prune(&self, ids: Vec<String>) -> Result<()> {
        match self {
            Self::Local(m) => m.prune(ids).await,
            Self::Remote(m) => m.prune(ids).await,
        }
    }

    async fn clear_cache(&self) -> Result<()> {
        match self {
            Self::Local(m) => m.clear_cache().await,
            Self::Remote(m) => m.clear_cache().await,
        }
    }

    async fn hibernate(&self) -> Result<()> {
        match self {
            Self::Local(m) => m.hibernate().await,
            Self::Remote(m) => m.hibernate().await,
        }
    }

    async fn wake(&self) -> Result<()> {
        match self {
            Self::Local(m) => m.wake().await,
            Self::Remote(m) => m.wake().await,
        }
    }
}

pub struct LocalVectorMemory {
    path: PathBuf,
    cold_path: PathBuf,
    embedder: Arc<RwLock<Option<TextEmbedding>>>,
    /// HOT Memory: All entries currently in RAM
    hot_entries: Arc<RwLock<Vec<MemoryEntry>>>,
    /// COLD Memory: Memory-mapped pool
    cold_cache: Arc<RwLock<Option<Vec<MemoryEntry>>>>,
}

impl LocalVectorMemory {
    pub fn new(path: PathBuf) -> Result<Self> {
        let cold_path = path.with_extension("cold");
        let embedder = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        ).context("Failed to initialize embedding model")?;

        let mut instance = Self {
            path,
            cold_path,
            embedder: Arc::new(RwLock::new(Some(embedder))),
            hot_entries: Arc::new(RwLock::new(Vec::new())),
            cold_cache: Arc::new(RwLock::new(None)),
        };

        instance.load()?;
        Ok(instance)
    }

    fn load(&mut self) -> Result<()> {
        if self.path.exists() {
            let file = File::open(&self.path)?;
            let decoder = match zstd::stream::read::Decoder::new(file) {
                Ok(d) => d,
                Err(e) => {
                    error!("Memory Corruption Detected (Zstd init): {}", e);
                    return self.recover_corrupt();
                }
            };
            
            match bincode::deserialize_from::<_, Vec<MemoryEntry>>(decoder) {
                Ok(entries) => {
                    info!("Loaded {} memories into HOT cache", entries.len());
                    *self.hot_entries.blocking_write() = entries;
                },
                Err(e) => {
                    error!("Memory Corruption Detected (Deserialize): {}", e);
                    return self.recover_corrupt();
                }
            }
        }
        Ok(())
    }

    fn recover_corrupt(&self) -> Result<()> {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let corrupt_path = self.path.with_extension(format!("corrupt.{}", timestamp));
        
        if let Err(e) = std::fs::rename(&self.path, &corrupt_path) {
            error!("Failed to rename corrupt memory file: {}", e);
        } else {
            error!("⚠️ Corrupt memory file moved to {:?}. Starting fresh.", corrupt_path);
        }
        Ok(())
    }

    async fn ensure_cold_cache(&self) -> Result<()> {
        let mut cache = self.cold_cache.write().await;
        if cache.is_none() && self.cold_path.exists() {
            debug!("Mmap: Mapping COLD memory into address space...");
            let file = File::open(&self.cold_path)?;
            let mmap = unsafe { Mmap::map(&file)? };
            let entries: Vec<MemoryEntry> = bincode::deserialize(&mmap[..])?;
            *cache = Some(entries);
        } else if cache.is_none() {
            *cache = Some(Vec::new());
        }
        Ok(())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut embedder_lock = self.embedder.write().await;
        if embedder_lock.is_none() {
            *embedder_lock = Some(TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?);
        }
        let embedder = embedder_lock.as_mut().unwrap();
        let mut embeddings = embedder.embed(texts.to_vec(), None)?;
        for emb in &mut embeddings { Self::normalize(emb); }
        Ok(embeddings)
    }

    fn normalize(vec: &mut Vec<f32>) {
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 { for x in vec { *x /= norm; } }
    }

    fn dot_product(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }
}

#[async_trait]
impl Memory for LocalVectorMemory {
    async fn store(&self, mut entry: MemoryEntry) -> Result<String> {
        if entry.embedding.is_none() {
            let embeddings = self.embed(&[entry.content.clone()]).await?;
            entry.embedding = Some(embeddings[0].clone());
        }
        
        let mut hot = self.hot_entries.write().await;
        hot.retain(|e| e.id != entry.id);
        
        let id = entry.id.clone();
        hot.push(entry);
        Ok(id)
    }

    async fn search(&self, query: &str, top_k: usize, context: Option<&str>, kind: Option<crate::orchestrator::Kind>) -> Result<Vec<MemoryEntry>> {
        let query_embedding = self.embed(&[query.to_string()]).await?.into_iter().next().context("No embedding")?;
        self.ensure_cold_cache().await?;

        let hot = self.hot_entries.read().await;
        let cold_guard = self.cold_cache.read().await;
        let cold = cold_guard.as_ref().unwrap();

        // Parallel Search over BOTH Tiers simultaneously via Rayon
        let mut all_results: Vec<(f32, MemoryEntry)> = hot.par_iter()
            .chain(cold.par_iter())
            .filter(|e| {
                let ctx_m = context.map_or(true, |c| e.metadata.context == c);
                let kind_m = kind.as_ref().map_or(true, |k| &e.metadata.kind == k);
                ctx_m && kind_m
            })
            .filter_map(|e| {
                e.embedding.as_ref().map(|emb| (Self::dot_product(&query_embedding, emb), e.clone()))
            })
            .collect();

        all_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        
        let mut final_entries: Vec<MemoryEntry> = all_results.into_iter().take(top_k).map(|(s, mut e)| {
            e.similarity = Some(s);
            e.metadata.access_count += 1;
            e
        }).collect();

        Ok(final_entries)
    }

    async fn count(&self) -> Result<usize> { 
        let hot = self.hot_entries.read().await.len();
        self.ensure_cold_cache().await?;
        let cold = self.cold_cache.read().await.as_ref().unwrap().len();
        Ok(hot + cold)
    }
    
    async fn persist(&self) -> Result<()> {
        let hot = self.hot_entries.read().await;
        let path = self.path.clone();
        let hot_clone = hot.clone(); 

        tokio::task::spawn_blocking(move || {
            let file = File::create(path)?;
            let writer = BufWriter::new(file);
            let mut encoder = zstd::stream::write::Encoder::new(writer, 3)?;
            bincode::serialize_into(&mut encoder, &hot_clone)?;
            encoder.finish()?;
            Ok::<(), anyhow::Error>(())
        }).await??;
        
        Ok(())
    }

    async fn consolidate(&self) -> Result<usize> {
        let mut hot = self.hot_entries.write().await;
        if hot.len() < 50 { return Ok(0); }

        info!("🧠 Memory Metabolism: Moving cold experiences to mmap storage...");

        let (stay_hot, to_cold): (Vec<_>, Vec<_>) = hot.drain(..).partition(|e| {
            e.metadata.access_count > 5 || e.metadata.importance > 0.8
        });

        let moved_count = to_cold.len();
        *hot = stay_hot;

        // Append to COLD binary file
        self.ensure_cold_cache().await?;
        let mut cold_guard = self.cold_cache.write().await;
        let cold = cold_guard.as_mut().unwrap();
        cold.extend(to_cold);

        // Persist COLD tier
        let cold_clone = cold.clone();
        let cold_path = self.cold_path.clone();
        tokio::task::spawn_blocking(move || {
            let file = OpenOptions::new().create(true).write(true).truncate(true).open(cold_path)?;
            bincode::serialize_into(file, &cold_clone)?;
            Ok::<(), anyhow::Error>(())
        }).await??;

        info!("🧠 Consolidation complete: Moved {} memories to COLD tier.", moved_count);
        Ok(moved_count)
    }

    async fn get_cold_memories(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        let hot = self.hot_entries.read().await;
        let mut cold: Vec<_> = hot.iter()
            .filter(|e| e.metadata.access_count <= 2 && e.metadata.importance < 0.7)
            .cloned()
            .collect();
        cold.truncate(limit);
        Ok(cold)
    }

    async fn get_recent(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        let hot = self.hot_entries.read().await;
        let mut recent: Vec<_> = hot.iter().cloned().collect();
        // HOT entries are appended, so last is newest.
        recent.reverse();
        recent.truncate(limit);
        Ok(recent)
    }

    async fn prune(&self, ids: Vec<String>) -> Result<()> {
        let mut hot = self.hot_entries.write().await;
        hot.retain(|e| !ids.contains(&e.id));
        Ok(())
    }
    
    async fn clear_cache(&self) -> Result<()> { 
        *self.cold_cache.write().await = None;
        Ok(()) 
    }
    
    async fn hibernate(&self) -> Result<()> {
        *self.embedder.write().await = None;
        *self.cold_cache.write().await = None;
        Ok(())
    }
    
    async fn wake(&self) -> Result<()> {
        self.ensure_cold_cache().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::entry::MemorySource;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_local_memory_tiering_logic() -> Result<()> {
        std::env::set_var("AGENCY_USE_REMOTE_MEMORY", "0");
        std::env::set_var("ORT_STRATEGY", "download");
        
        // Skip if ONNX lib missing to prevent process-wide panics in CI
        if std::env::var("ORT_DYLIB_PATH").is_err() && !std::path::Path::new("libonnxruntime.dylib").exists() {
            return Ok(());
        }

        let dir = tempdir()?;
        let path = dir.path().join("test.mem");
        let memory = LocalVectorMemory::new(path)?;

        // 1. Fill HOT cache manually (bypassing embedding for logic test)
        {
            let mut hot = memory.hot_entries.write().await;
            for i in 0..100 {
                let mut entry = MemoryEntry::new(format!("Memory {}", i), "test", MemorySource::User);
                // Make some 'Cold' (low access, low importance)
                if i < 80 {
                    entry.metadata.access_count = 0;
                    entry.metadata.importance = 0.1;
                } else {
                    entry.metadata.access_count = 10;
                    entry.metadata.importance = 0.9;
                }
                hot.push(entry);
            }
        }

        // 2. Consolidate
        let moved = memory.consolidate().await?;
        assert!(moved > 0, "Should have moved items to cold tier");
        
        let hot_count = memory.hot_entries.read().await.len();
        let cold_count = memory.count().await? - hot_count;
        
        assert!(hot_count < 100);
        assert!(cold_count > 0);
        assert!(memory.cold_path.exists(), "Cold file should be created");

        Ok(())
    }
}

pub struct RemoteVectorMemory {
    client: Client,
    url: String,
}

impl RemoteVectorMemory {
    pub fn new(url: String) -> Self {
        Self { client: Client::new(), url }
    }
}

#[async_trait]
impl Memory for RemoteVectorMemory {
    async fn store(&self, entry: MemoryEntry) -> Result<String> {
        let resp = self.client.post(format!("{}/store", self.url))
            .json(&json!({ "entry": entry }))
            .send().await?;
        let data: serde_json::Value = resp.json().await?;
        Ok(data["id"].as_str().context("No ID in response")?.to_string())
    }

    async fn search(&self, query: &str, top_k: usize, context: Option<&str>, kind: Option<crate::orchestrator::Kind>) -> Result<Vec<MemoryEntry>> {
        let resp = self.client.post(format!("{}/search", self.url))
            .json(&json!({
                "query": query,
                "top_k": top_k,
                "context": context,
                "kind": kind
            }))
            .send().await?;
        let data: serde_json::Value = resp.json().await?;
        let entries = serde_json::from_value(data["entries"].clone())?;
        Ok(entries)
    }

    async fn count(&self) -> Result<usize> {
        let resp = self.client.get(format!("{}/count", self.url)).send().await?;
        let data: serde_json::Value = resp.json().await?;
        Ok(data["count"].as_u64().unwrap_or(0) as usize)
    }

    async fn persist(&self) -> Result<()> {
        self.client.post(format!("{}/persist", self.url)).send().await?;
        Ok(())
    }

    async fn consolidate(&self) -> Result<usize> { Ok(0) }
    async fn get_cold_memories(&self, _limit: usize) -> Result<Vec<MemoryEntry>> { Ok(Vec::new()) }
    async fn get_recent(&self, _limit: usize) -> Result<Vec<MemoryEntry>> { Ok(Vec::new()) }
    async fn prune(&self, _ids: Vec<String>) -> Result<()> { Ok(()) }
    async fn clear_cache(&self) -> Result<()> { Ok(()) }
    async fn hibernate(&self) -> Result<()> { Ok(()) }
    async fn wake(&self) -> Result<()> { Ok(()) }
}
