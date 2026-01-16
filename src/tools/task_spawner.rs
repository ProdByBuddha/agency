//! Task Spawner Tool
//! 
//! Allows agents to spawn new background tasks into the persistent queue.
//! This enables "Cellular Division" of complex goals.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};
use crate::orchestrator::queue::TaskQueue;

pub struct TaskSpawnerTool {
    queue: Arc<dyn TaskQueue>,
}

impl TaskSpawnerTool {
    pub fn new(queue: Arc<dyn TaskQueue>) -> Self {
        Self { queue }
    }
}

#[async_trait]
impl Tool for TaskSpawnerTool {
    fn name(&self) -> String {
        "spawn_task".to_string()
    }

    fn description(&self) -> String {
        "Spawn a new background task. Use this to break down complex goals into smaller, parallelizable sub-tasks. The task will be executed asynchronously.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The description of the sub-task to perform."
                }
            },
            "required": ["goal"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let goal = params["goal"].as_str()
            .ok_or_else(|| AgentError::Execution("Missing 'goal' parameter".to_string()))?;

        // We wrap the goal in the standard payload structure
        let payload = json!(goal);
        
        match self.queue.enqueue("autonomous_goal", payload).await {
            Ok(id) => Ok(ToolOutput::success(
                json!({ "task_id": id, "status": "queued" }), 
                format!("Task spawned successfully. ID: {}", id)
            )),
            Err(e) => Ok(ToolOutput::failure(format!("Failed to spawn task: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::queue::SqliteTaskQueue;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_task_spawner_execution() {
        let tmp = NamedTempFile::new().unwrap();
        let queue = Arc::new(SqliteTaskQueue::new(tmp.path()).await.unwrap());
        let tool = TaskSpawnerTool::new(queue.clone());

        let res = tool.execute(json!({
            "goal": "Test spawning a single sub-task"
        })).await.unwrap();

        assert!(res.success);
        assert_eq!(queue.count("pending").await.unwrap(), 1);
    }
}
