//! Notify Tool
//! 
//! Allows agents to explicitly send a message to the user's mobile device
//! via the Vocal Cords (Telegram).

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};
use crate::orchestrator::vocal_cords::VocalCords;

pub struct NotifyTool {
    vocal_cords: Arc<VocalCords>,
}

impl NotifyTool {
    pub fn new(vocal_cords: Arc<VocalCords>) -> Self {
        Self { vocal_cords }
    }
}

#[async_trait]
impl Tool for NotifyTool {
    fn name(&self) -> String {
        "notify_user".to_string()
    }

    fn description(&self) -> String {
        "Send a high-priority notification message to the user's mobile device. Use this for important results, critical alerts, or when a task is completed while the user is away.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to send."
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let message = params["message"].as_str()
            .ok_or_else(|| AgentError::Execution("Missing 'message'".to_string()))?;

        if !self.vocal_cords.is_active() {
            return Ok(ToolOutput::failure("Vocal Cords are not active. Ensure TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID are set."));
        }

        match self.vocal_cords.say(message).await {
            Ok(_) => Ok(ToolOutput::success(json!({"status": "sent"}), "Notification sent successfully.")),
            Err(e) => Ok(ToolOutput::failure(format!("Failed to send notification: {}", e))),
        }
    }
}
