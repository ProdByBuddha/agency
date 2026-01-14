//! Agent Module
//! 
//! Provides the ReAct agent framework with specialized agent types.

mod react;
mod reflection;
mod types;
mod autonomous;
mod background;
mod provider;
mod ctm;
mod cache;
pub mod nqd;
pub mod speaker_rs;
pub mod rl;
pub mod training;

pub use speaker_rs::Speaker;
pub use react::{ReActAgent, ReActStep, AgentResponse, SimpleAgent};
pub use reflection::Reflector;
pub use types::{AgentType, AgentConfig};
pub use autonomous::AutonomousMachine;
pub use background::BackgroundThoughtMachine;
pub use ctm::ContinuousThoughtMachine;
pub use provider::{LLMProvider, OllamaProvider, OpenAICompatibleProvider, CandleProvider, RemoteNexusProvider, PublishingProvider};
pub use cache::{LLMCache, CachedProvider};
pub use nqd::NQDPortfolio;

use async_trait::async_trait;
use thiserror::Error;
use serde::{Serialize, Deserialize};

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("LLM Provider error: {0}")]
    Provider(String),
    #[error("Parsing error: {0}")]
    Parse(String),
    #[error("Tool execution error: {0}")]
    Tool(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("PAI Core error: {0}")]
    Pai(String),
    #[error("Execution failed: {0}")]
    Execution(String),
}

/// Specialized Result for Agent operations
pub type AgentResult<T> = std::result::Result<T, AgentError>;

/// FPF Boundary Norm Quadrants (A.6.B)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LadeQuadrant {
    /// L - Laws & Definitions (Truth-conditional content)
    L,
    /// A - Admissibility & Gates (Runtime entry predicates)
    A,
    /// D - Deontics & Commitments (Governance/Obligations)
    D,
    /// E - Work-Effects & Evidence (Observed outcomes)
    E,
}

/// FPF Publication Characteristics (E.17.5.5)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubCharacteristic {
    pub pc_type: String, // PC.Number, PC.EvidenceBinding, etc.
    pub value: serde_json::Value,
    pub unit: Option<String>,
    pub scale: Option<String>,
    pub reference_plane: Option<String>,
    pub edition: String,
}

/// Trait for specialized agents
#[async_trait]
pub trait Agent: Send + Sync {
    /// Get the agent type
    #[allow(dead_code)]
    fn agent_type(&self) -> AgentType;
    
    /// Get the agent's name
    #[allow(dead_code)]
    fn name(&self) -> &str;
    
    /// Get the agent's system prompt
    #[allow(dead_code)]
    fn system_prompt(&self) -> &str;
    
    /// Get the model to use for this agent
    #[allow(dead_code)]
    fn model(&self) -> &str;
    
    /// Execute a query and return a response
    async fn execute(&self, query: &str, context: Option<&str>) -> AgentResult<AgentResponse>;
}

pub fn truncate(s: &str, max_len: usize) -> String {
    let s = s.replace('\n', " ");
    if s.len() <= max_len {
        s
    } else {
        let target_len = max_len.saturating_sub(3);
        let mut end = target_len;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Helper to check if a query likely requires tool use
pub fn is_action_query(query: &str) -> bool {
    let q = query.to_lowercase();
    let action_keywords = [
        "create", "write", "search", "find", "analyze", "list", "run", "execute", 
        "debug", "fix", "refactor", "index", "show", "what is in", "contents",
        "http://", "https://", ".com", ".org", ".net", ".io"
    ];
    action_keywords.iter().any(|&k| q.contains(k))
}

