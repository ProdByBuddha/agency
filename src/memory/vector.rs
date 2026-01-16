//! Vector Memory Implementation using Native Rust (Vec + Rayon)
//! 
//! Provides semantic search over stored memories using naive vector search
//! parallelized with Rayon. Persists to disk using Bincode for efficiency.
//! Supports local (embedded) or remote (microservice) modes.

use anyhow::{Context, Result};
use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use reqwest::Client;
use serde_json::json;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use rayon::prelude::*;

use super::{Memory, MemoryEntry, entry::MemorySource};

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
            info!("Initializing LocalVectorMemory (Native) at {:?}", path);
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

/// Vector memory backed by local file storage (Bincode)
pub struct LocalVectorMemory {
    path: PathBuf,
    embedder: Arc<RwLock<Option<TextEmbedding>>>,
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

        // Load if exists (Bincode or fallback to JSON for legacy compat)
        instance.load()?;

        Ok(instance)
    }

    fn load(&mut self) -> Result<()> {
        if self.path.exists() {
            // Try Bincode first
            if let Ok(file) = File::open(&self.path) {
                let reader = BufReader::new(file);
                match bincode::deserialize_from::<_, Vec<MemoryEntry>>(reader) {
                    Ok(entries) => {
                        info!("Loaded {} memories from binary store", entries.len());
                        *self.entries.blocking_write() = entries;
                        return Ok(());
                    },
                    Err(_) => {
                        // Fallback to JSON (migration path)
                        let content = std::fs::read_to_string(&self.path)?;
                        if let Ok(entries) = serde_json::from_str::<Vec<MemoryEntry>>(&content) {
                            info!("Loaded {} memories from legacy JSON store", entries.len());
                            *self.entries.blocking_write() = entries;
                            // We will save as bincode on next persist
                            return Ok(());
                        }
                    }
                }
            }
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
        
        // Remove duplicates if same ID (update)
        entries.retain(|e| e.id != entry.id);
        
        // CodebaseIndexer deduplication logic
        if let Some(ref query) = entry.query {
            if entry.metadata.agent == "CodebaseIndexer" {
                entries.retain(|e| e.query.as_ref() != Some(query));
            }
        }
        
        let id = entry.id.clone();
        entries.push(entry);
        
        // Auto-persist on write for durability
        // We do this in background or sync? Doing sync for safety.
        // But to avoid blocking async runtime, we should use spawn_blocking if file IO is heavy.
        // For now, we rely on the `persist` method being called or valid shutdown.
        // Actually, let's persist here to be safe (Muscular durability).
        // But serializing huge vec every time is slow.
        // We'll rely on explicit persist() call or periodic save.
        
        Ok(id)
    }

    async fn search(&self, query: &str, top_k: usize, context: Option<&str>, kind: Option<crate::orchestrator::Kind>) -> Result<Vec<MemoryEntry>> {
        let query_embedding = self.embed(&[query.to_string()]).await?.into_iter().next().context("No embedding")?;
        
        let entries_guard = self.entries.read().await;
        // Clone entries for parallel processing (cheap if Arc, but MemoryEntry is not Arc)
        // We process references in parallel
        
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

        // Sort descending by score
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        
        Ok(scored.into_iter().take(top_k).map(|(s, idx)| {
            let mut e = entries_guard[idx].clone();
            e.similarity = Some(s);
            e
        }).collect())
    }

    async fn count(&self) -> Result<usize> { Ok(self.entries.read().await.len()) }
    
    async fn persist(&self) -> Result<()> {
        let entries = self.entries.read().await;
        let path = self.path.clone();
        let entries_clone = entries.clone(); // Clone to move to thread
        
        tokio::task::spawn_blocking(move || {
            let file = File::create(path)?;
            let writer = BufWriter::new(file);
            bincode::serialize_into(writer, &entries_clone)?;
            Ok::<(), anyhow::Error>(())
        }).await??;
        
        Ok(())
    }
    
    async fn clear_cache(&self) -> Result<()> { 
        // We don't clear entries as they are the DB
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_entry_creation() {
        let entry = MemoryEntry::new("Test content", "TestAgent", MemorySource::Agent);
        assert!(!entry.id.is_empty());
        assert_eq!(entry.content, "Test content");
        assert_eq!(entry.metadata.agent, "TestAgent");
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((LocalVectorMemory::dot_product(&a, &b) - 1.0).abs() < 0.001);
        
        let c = vec![0.0, 1.0, 0.0];
        assert!(LocalVectorMemory::dot_product(&a, &c).abs() < 0.001);
    }
}