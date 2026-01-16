//! Memory System Module
//! 
//! Provides semantic vector memory using fastembed,
//! plus episodic conversation history.

pub mod vector;
pub mod episodic;
pub mod entry;
pub mod manager;
pub mod indexer;
pub mod history;
pub mod compactor;

pub use vector::{VectorMemory, LocalVectorMemory, RemoteVectorMemory};
pub use episodic::EpisodicMemory;
pub use entry::MemoryEntry;
pub use manager::MemoryManager;
pub use indexer::CodebaseIndexer;
pub use history::{HistoryManager, HistoryEntry};
pub use compactor::ContextCompactor;

use anyhow::Result;
use async_trait::async_trait;

/// Trait for memory systems that can store and retrieve entries
#[async_trait]
pub trait Memory: Send + Sync {
    /// Store a new memory entry
    async fn store(&self, entry: MemoryEntry) -> Result<String>;
    
    /// Search for relevant memories based on a query
    async fn search(&self, query: &str, top_k: usize, context: Option<&str>, kind: Option<crate::orchestrator::Kind>) -> Result<Vec<MemoryEntry>>;
    
    /// Get total number of entries
    #[allow(dead_code)]
    async fn count(&self) -> Result<usize>;

    /// Persist memory to disk
    async fn persist(&self) -> Result<()>;

    /// Consolidate cold memories (dreaming)
    async fn consolidate(&self) -> Result<usize>;

    /// Clear transient caches to free up RAM
    #[allow(dead_code)]
    async fn clear_cache(&self) -> Result<()>;

    /// Hibernate the memory system (unload heavy models/caches)
    async fn hibernate(&self) -> Result<()>;

    /// Wake the memory system (reload models/caches)
    async fn wake(&self) -> Result<()>;
}
