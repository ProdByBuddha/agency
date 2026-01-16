//! Knowledge Graph Tool - Visualizes distilled relationships
//! 
//! Queries the vector memory for entity relationships and formats them
//! as Mermaid diagrams for visualization.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::{AgentResult, AgentError};
use crate::memory::Memory;
use crate::tools::{Tool, ToolOutput};

/// Tool for visualizing distilled knowledge
pub struct KnowledgeGraphTool {
    memory: Arc<dyn Memory>,
}

impl KnowledgeGraphTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    fn parse_triple(&self, content: &str) -> Option<(String, String, String)> {
        // Expected format: "[Entity Name] -> [Relationship] -> [Target]"
        let parts: Vec<&str> = content.split("->").map(|s| s.trim()).collect();
        if parts.len() == 3 {
            Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
        } else {
            None
        }
    }
}

#[async_trait]
impl Tool for KnowledgeGraphTool {
    fn name(&self) -> String {
        "knowledge_graph_viewer".to_string()
    }

    fn description(&self) -> String {
        "Visualizes distilled entity relationships from memory as a Mermaid diagram. Use this to understand the high-level connections the agency has learned.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of relationships to show",
                    "default": 20
                }
            }
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "constrained",
            "environment": "internal memory graph",
            "search_mode": "topological (entity-relationship)",
            "data_scope": "local knowledge base"
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let limit = params["limit"].as_u64().unwrap_or(20) as usize;

        // Search for knowledge graph entries (across all contexts for now)
        let results = self.memory.search("entity relationship knowledge graph distilled", limit * 5, None, None).await
            .map_err(|e| AgentError::Tool(format!("Memory search failed: {}", e)))?;
        
        let mut triples = Vec::new();
        for entry in results {
            if entry.metadata.tags.contains(&"knowledge_graph".to_string()) {
                if let Some(triple) = self.parse_triple(&entry.content) {
                    triples.push(triple);
                }
            }
        }

        if triples.is_empty() {
            return Ok(ToolOutput::success(
                json!({"mermaid": "", "triples_count": 0}),
                "No distilled relationships found in memory yet. Continue interacting to allow the agency to learn!"
            ));
        }

        // De-duplicate and limit
        triples.sort();
        triples.dedup();
        let triples: Vec<_> = triples.into_iter().take(limit).collect();

        // Generate Mermaid
        let mut mermaid = String::from("graph TD\n");
        for (sub, pred, obj) in &triples {
            let s = sub.replace(' ', "_").replace('-', "_");
            let o = obj.replace(' ', "_").replace('-', "_");
            mermaid.push_str(&format!("    {} -- \"{}\" --> {}\n", s, pred, o));
        }

        let summary = format!(
            "Generated knowledge graph with {} relationships.\n\n```mermaid\n{}\n```",
            triples.len(),
            mermaid
        );

        Ok(ToolOutput::success(
            json!({
                "mermaid": mermaid,
                "triples_count": triples.len(),
                "triples": triples
            }),
            summary
        ))
    }
}
