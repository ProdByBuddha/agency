//! Provider Management Tool
//! 
//! Allows the Agency to switch LLM backends (providers) at runtime.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::agent::{AgentResult, AgentError, provider::{SwitchableProvider, create_provider_by_type}};
use crate::tools::{Tool, ToolOutput};

pub struct ProviderTool {
    provider: Arc<SwitchableProvider>,
}

impl ProviderTool {
    pub fn new(provider: Arc<SwitchableProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl Tool for ProviderTool {
    fn name(&self) -> String {
        "provider_manager".to_string()
    }

    fn description(&self) -> String {
        "Switch the active LLM provider (e.g., 'zai', 'ollama-cloud', 'ollama', 'openai', 'candle').".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["switch", "status"],
                    "description": "The action to perform."
                },
                "provider_type": {
                    "type": "string",
                    "description": "The target provider type (only for 'switch')."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let action = params["action"].as_str().unwrap_or("status");

        match action {
            "switch" => {
                let target = params["provider_type"].as_str()
                    .ok_or_else(|| AgentError::Tool("Missing 'provider_type' for switch action".to_string()))?;
                
                let new_inner = create_provider_by_type(target);
                self.provider.switch_to(new_inner).await;
                
                Ok(ToolOutput::success(
                    json!({ "new_provider": target }),
                    format!("Successfully switched LLM provider to '{}'. Models will now map to this backend.", target)
                ))
            },
            "status" => {
                Ok(ToolOutput::success(
                    json!({ "status": "active" }),
                    "Provider manager is active. You can switch to 'zai', 'ollama-cloud', 'ollama', or 'candle'.".to_string()
                ))
            },
            _ => Err(AgentError::Tool(format!("Unknown action: {}", action)))
        }
    }
}
