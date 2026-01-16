//! Memory Query Tool
//! 
//! Allows agents to search their own memory.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

use crate::agent::{AgentResult, AgentError};
use super::{Tool, ToolOutput};
use crate::memory::Memory;

/// Tool for querying the memory system
pub struct MemoryQueryTool {
    memory: Arc<dyn Memory>,
}

impl MemoryQueryTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> String {
        "memory_query".to_string()
    }

    fn description(&self) -> String {
        "Search your memory for past interactions, learned information, or context. \
         Use this when you need to recall previous conversations or find relevant information \
         from past interactions.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to find relevant memories"
                },
                "top_k": {
                    "type": "integer",
                    "description": "Number of results to return (default: 3, max: 10)",
                    "default": 3
                }
            },
            "required": ["query"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "constrained",
            "environment": "internal vector database",
            "search_mode": "semantic similarity",
            "data_scope": "local long-term memory"
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| AgentError::Validation("Missing required parameter: query".to_string()))?;
        
        let top_k = params["top_k"]
            .as_u64()
            .unwrap_or(3)
            .min(10) as usize;
        let kind_str = params["kind"].as_str();
        
        let kind = if let Some(k) = kind_str {
            match k.to_lowercase().as_str() {
                "technical" => Some(crate::orchestrator::Kind::Technical),
                "evidence" => Some(crate::orchestrator::Kind::Evidence),
                "strategic" => Some(crate::orchestrator::Kind::Strategic),
                "operational" => Some(crate::orchestrator::Kind::Operational),
                "governance" => Some(crate::orchestrator::Kind::Governance),
                _ => None,
            }
        } else {
            None
        };

        debug!("Querying memory for: {} (top {}, kind: {:?})", query, top_k, kind);

        match self.memory.search(query, top_k, None, kind).await {
            Ok(entries) => {
                if entries.is_empty() {
                    return Ok(ToolOutput::success_str(
                        "No relevant memories found for this query."
                    ));
                }

                let formatted = entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let similarity = e.similarity
                            .map(|s| format!(" (relevance: {:.2})", s))
                            .unwrap_or_default();
                        
                        format!(
                            "{}. [{}{}]\n   {}\n   Time: {}",
                            i + 1,
                            e.metadata.agent,
                            similarity,
                            e.content.chars().take(500).collect::<String>(),
                            e.timestamp.format("%Y-%m-%d %H:%M")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");

                let summary = format!(
                    "Found {} relevant memories:\n\n{}",
                    entries.len(),
                    formatted
                );

                Ok(ToolOutput::success(
                    json!({
                        "query": query,
                        "num_results": entries.len(),
                        "memories": entries.iter().map(|e| json!({
                            "id": e.id,
                            "content": e.content,
                            "agent": e.metadata.agent,
                            "timestamp": e.timestamp.to_rfc3339(),
                            "similarity": e.similarity
                        })).collect::<Vec<_>>()
                    }),
                    summary
                ))
            }
            Err(e) => {
                Ok(ToolOutput::failure(format!("Memory search failed: {}", e)))
            }
        }
    }
}
