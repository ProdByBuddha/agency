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

pub use react::{ReActAgent, ReActStep, AgentResponse, SimpleAgent};
pub use reflection::Reflector;
pub use types::{AgentType, AgentConfig};
pub use autonomous::AutonomousMachine;
pub use background::BackgroundThoughtMachine;
pub use ctm::ContinuousThoughtMachine;
pub use provider::{LLMProvider, OllamaProvider, OpenAICompatibleProvider};
pub use cache::{LLMCache, CachedProvider};

use anyhow::Result;
use async_trait::async_trait;

/// Trait for specialized agents
#[async_trait]
pub trait Agent: Send + Sync {
    /// Get the agent type
    fn agent_type(&self) -> AgentType;
    
    /// Get the agent's name
    fn name(&self) -> &str;
    
    /// Get the agent's system prompt
    fn system_prompt(&self) -> &str;
    
    /// Get the model to use for this agent
    fn model(&self) -> &str;
    
    /// Execute a query and return a response
    async fn execute(&self, query: &str, context: Option<&str>) -> Result<AgentResponse>;
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

pub fn is_action_query(query: &str) -> bool {
    let q = query.to_lowercase();
    let action_keywords = [
        "create", "write", "search", "analyze", "run", "execute", "list", 
        "find", "build", "forge", "calculate", "read", "check", "entropy"
    ];
    action_keywords.iter().any(|k| q.contains(k))
}

