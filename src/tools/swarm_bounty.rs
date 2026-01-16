//! Swarm Bounty Tool
//! 
//! Allows agents to broadcast a task to the anonymous global swarm (Hive Mind).
//! This enqueues a persistent task that the Hive Worker will broadcast over Tor.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};
use crate::orchestrator::queue::TaskQueue;

pub struct SwarmBountyTool {
    queue: Arc<dyn TaskQueue>,
}

impl SwarmBountyTool {
    pub fn new(queue: Arc<dyn TaskQueue>) -> Self {
        Self { queue }
    }
}

#[async_trait]
impl Tool for SwarmBountyTool {
    fn name(&self) -> String {
        "broadcast_to_swarm".to_string()
    }

        fn description(&self) -> String {
            "Broadcast a difficult task to the global anonymous swarm via Tor. Use this when you are stuck, need a second opinion, or lack the specialized knowledge to complete a goal. The swarm will process it asynchronously and the result will appear in your memory once completed.".to_string()
        }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The description of the task you need help with."
                },
                "priority": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "default": 5,
                    "description": "How urgent this task is for your mission."
                }
            },
            "required": ["goal"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "broadcast",
            "network": "tor/arti",
            "anonymity": "total",
            "reliability": "best-effort (swarm-dependent)"
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let goal = params["goal"].as_str()
            .ok_or_else(|| AgentError::Validation("Missing 'goal'".to_string()))?;
        let priority = params["priority"].as_u64().unwrap_or(5);

        // We enqueue a special 'swarm_bounty' task
        let payload = json!({
            "goal": goal,
            "priority": priority,
            "origin_agent": "local_supervisor"
        });
        
        match self.queue.enqueue("swarm_bounty", payload).await {
            Ok(id) => Ok(ToolOutput::success(
                json!({ "bounty_id": id, "status": "broadcast_pending" }), 
                format!("Bounty successfully broadcast to the local Hive Queue. ID: {}. The swarm will now begin anonymous consultation over Tor.", id)
            )),
            Err(e) => Ok(ToolOutput::failure(format!("Failed to enqueue swarm bounty: {}", e))),
        }
    }
}
