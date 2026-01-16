//! Vector Memory Implementation with Cognitive Tiering
//! 
//! Provides semantic search over stored memories using naive vector search
//! parallelized with Rayon. Persists to disk using Bincode + Zstd compression.
//! Supports local (embedded) or remote (microservice) modes.

use anyhow::{Context, Result};
use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};
use reqwest::Client;
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use rayon::prelude::*;

use super::{Memory, MemoryEntry};

/// Vector memory abstraction supporting local or remote backends
pub enum VectorMemory {
    Local(LocalVectorMemory),
    Remote(RemoteVectorMemory),
}

impl VectorMemory {
    /// Create a new VectorMemory instance based on environment config
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

/// Vector memory backed by local file storage (Bincode + Zstd)
pub struct LocalVectorMemory {
    path: PathBuf,
    embedder: Arc<RwLock<Option<TextEmbedding>>>,
    /// HOT Memory: All entries currently in RAM
    entries: Arc<RwLock<Vec<MemoryEntry>>>,
}

impl LocalVectorMemory {
    pub fn new(path: PathBuf) -> Result<Self> {
        let embedder = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        ).context("Failed to initialize embedding model")?;

        let mut instance = Self {
            path,
            embedder: Arc::new(RwLock::new(Some(embedder))),
            entries: Arc::new(RwLock::new(Vec::new())),
        };

        // Load if exists (Bincode or Zstd)
        instance.load()?;

        Ok(instance)
    }

    fn load(&mut self) -> Result<()> {
        if self.path.exists() {
            let file = File::open(&self.path)?;
            let mut reader = BufReader::new(file);
            
            // Peek for Zstd Magic Number
            let mut magic = [0u8; 4];
            let _ = reader.read(&mut magic);
            
            let file = File::open(&self.path)?; 
            let reader = BufReader::new(file);

            let entries = if magic == [0x28, 0xB5, 0x2F, 0xFD] {
                debug!("Memory: Loading compressed Zstd binary store");
                let decoder = zstd::stream::read::Decoder::new(reader)?;
                bincode::deserialize_from::<_, Vec<MemoryEntry>>(decoder)?
            } else {
                debug!("Memory: Loading legacy uncompressed store");
                bincode::deserialize_from::<_, Vec<MemoryEntry>>(reader)
                    .or_else(|_| {
                        let content = std::fs::read_to_string(&self.path)?;
                        serde_json::from_str::<Vec<MemoryEntry>>(&content)
                            .map_err(|e| anyhow::anyhow!("Failed to parse memory: {}", e))
                    })?
            };

            info!("Loaded {} memories into HOT cache", entries.len());
            *self.entries.blocking_write() = entries;
        }
        Ok(())
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        {
            let read_guard = self.embedder.read().await;
            if read_guard.is_none() {
                drop(read_guard);
                let mut write_guard = self.embedder.write().await;
                if write_guard.is_none() {
                    *write_guard = Some(TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?);
                }
            }
        }
        let mut embedder_lock = self.embedder.write().await;
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
        
        let mut entries = self.entries.write().await;
        entries.retain(|e| e.id != entry.id);
        
        if let Some(ref query) = entry.query {
            if entry.metadata.agent == "CodebaseIndexer" {
                entries.retain(|e| e.query.as_ref() != Some(query));
            }
        }
        
        let id = entry.id.clone();
        entries.push(entry);
        
        Ok(id)
    }

    async fn search(&self, query: &str, top_k: usize, context: Option<&str>, kind: Option<crate::orchestrator::Kind>) -> Result<Vec<MemoryEntry>> {
        let query_embedding = self.embed(&[query.to_string()]).await?.into_iter().next().context("No embedding")?;
        
        let mut entries_guard = self.entries.write().await;
        
        let mut scored: Vec<(f32, usize)> = entries_guard.par_iter().enumerate()
            .filter(|(_, e)| {
                let ctx_m = context.map_or(true, |c| e.metadata.context == c);
                let kind_m = kind.as_ref().map_or(true, |k| &e.metadata.kind == k);
                ctx_m && kind_m
            })
            .filter_map(|(idx, e)| {
                e.embedding.as_ref().map(|emb| (Self::dot_product(&query_embedding, emb), idx))
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        
        let results: Vec<MemoryEntry> = scored.into_iter().take(top_k).map(|(s, idx)| {
            entries_guard[idx].metadata.access_count += 1;
            let mut e = entries_guard[idx].clone();
            e.similarity = Some(s);
            e
        }).collect();

        Ok(results)
    }

    async fn count(&self) -> Result<usize> { Ok(self.entries.read().await.len()) }
    
    async fn persist(&self) -> Result<()> {
        let entries = self.entries.read().await;
        let path = self.path.clone();
        let entries_clone = entries.clone(); 
        
        info!("💾 Memory: Persisting {} entries with Zstd compression...", entries_clone.len());

        tokio::task::spawn_blocking(move || {
            let file = File::create(path)?;
            let writer = BufWriter::new(file);
            let encoder = zstd::stream::write::Encoder::new(writer, 3)?.auto_finish();
            bincode::serialize_into(encoder, &entries_clone)?;
            Ok::<(), anyhow::Error>(())
        }).await??;
        
        Ok(())
    }

    async fn consolidate(&self) -> Result<usize> {
        let mut entries = self.entries.write().await;
        let original_count = entries.len();
        
        if original_count < 100 {
            return Ok(0); 
        }

        info!("🧠 Memory Dreaming: Performing metabolic cleanup...");

        let now = chrono::Utc::now();
        let week_ago = now - chrono::Duration::days(7);
        
        let (hot, cold): (Vec<_>, Vec<_>) = entries.drain(..).partition(|e| {
            e.metadata.access_count > 5 || e.timestamp > week_ago || e.metadata.importance > 0.8
        });

        let cold_count = cold.len();
        *entries = hot;
        
        info!("🧠 Dreaming complete: Pruned {} cold memories from HOT cache.", cold_count);
        Ok(cold_count)
    }

    async fn get_cold_memories(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        let entries = self.entries.read().await;
        let now = chrono::Utc::now();
        let week_ago = now - chrono::Duration::days(7);

        let mut cold: Vec<_> = entries.iter()
            .filter(|e| e.metadata.access_count <= 2 && e.timestamp < week_ago && e.metadata.importance < 0.7)
            .cloned()
            .collect();

        cold.truncate(limit);
        Ok(cold)
    }

    async fn prune(&self, ids: Vec<String>) -> Result<()> {
        let mut entries = self.entries.write().await;
        entries.retain(|e| !ids.contains(&e.id));
        Ok(())
    }
    
    async fn clear_cache(&self) -> Result<()> { 
        Ok(()) 
    }
    
    async fn hibernate(&self) -> Result<()> {
        *self.embedder.write().await = None;
        Ok(())
    }
    
    async fn wake(&self) -> Result<()> {
        let mut emb = self.embedder.write().await;
        if emb.is_none() {
            *emb = Some(TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))?);
        }
        Ok(())
    }
}

/// Vector memory client for remote microservice
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

    async fn consolidate(&self) -> Result<usize> {
        Ok(0)
    }

    async fn get_cold_memories(&self, _limit: usize) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn prune(&self, _ids: Vec<String>) -> Result<()> {
        Ok(())
    }

    async fn clear_cache(&self) -> Result<()> {
        self.client.post(format!("{}/hibernate", self.url)).send().await?; 
        Ok(())
    }

    async fn hibernate(&self) -> Result<()> {
        self.client.post(format!("{}/hibernate", self.url)).send().await?;
        Ok(())
    }

    async fn wake(&self) -> Result<()> {
        self.client.post(format!("{}/wake", self.url)).send().await?;
        Ok(())
    }
}