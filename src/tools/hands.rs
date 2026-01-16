//! Hands Tool (GUI Control)
//! 
//! Provides the agency with a voice AND ears on external platforms.
//! Enables proactive notifications and remote command execution.

use async_trait::async_trait;
use serde_json::{json, Value};
use enigo::{Enigo, Mouse, Keyboard, Button, Direction, Coordinate, Key, Settings};
use tracing::{info, debug};
use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};

pub struct HandsTool;

impl HandsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for HandsTool {
    fn name(&self) -> String {
        "hands".to_string()
    }

    fn description(&self) -> String {
        "Direct GUI control. Move the mouse, click, and type text into the active window. \
         Use this to perform tasks in apps that don't have APIs. \
         ACTIONS: 'mouse_move', 'mouse_click', 'type_text', 'key_tap'.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["mouse_move", "mouse_click", "type_text", "key_tap"],
                    "description": "The GUI action to perform."
                },
                "x": { "type": "integer", "description": "X coordinate (for mouse_move)" },
                "y": { "type": "integer", "description": "Y coordinate (for mouse_move)" },
                "button": { "type": "string", "enum": ["left", "right"], "default": "left" },
                "text": { "type": "string", "description": "Text to type (for type_text)" },
                "key": { "type": "string", "description": "Special key name (e.g. 'enter', 'tab', 'escape')" }
            },
            "required": ["action"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "physical_impact",
            "environment": "macOS GUI",
            "safety": "CRITICAL (Requires visual grounding and human confirmation)",
            "requirements": ["manual_approval", "active_display"]
        })
    }

    fn requires_confirmation(&self) -> bool {
        true 
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let action = params["action"].as_str().ok_or_else(|| AgentError::Validation("Missing 'action'".to_string()))?.to_string();
        
        // We use spawn_blocking because Enigo is not Send/Sync on macOS 
        // and we want to perform synchronous UI actions.
        let result = tokio::task::spawn_blocking(move || -> Result<ToolOutput, anyhow::Error> {
            let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow::anyhow!("Enigo init failed: {}", e))?;
            
            match action.as_str() {
                "mouse_move" => {
                    let x = params["x"].as_i64().ok_or_else(|| anyhow::anyhow!("Missing 'x'"))? as i32;
                    let y = params["y"].as_i64().ok_or_else(|| anyhow::anyhow!("Missing 'y'"))? as i32;
                    enigo.move_mouse(x, y, Coordinate::Abs).map_err(|e| anyhow::anyhow!("Move error: {}", e))?;
                    Ok(ToolOutput::success(json!({"x": x, "y": y}), format!("Moved mouse to ({}, {})", x, y)))
                },
                "mouse_click" => {
                    let btn_str = params["button"].as_str().unwrap_or("left");
                    let btn = if btn_str == "right" { Button::Right } else { Button::Left };
                    enigo.button(btn, Direction::Click).map_err(|e| anyhow::anyhow!("Click error: {}", e))?;
                    Ok(ToolOutput::success(json!({"button": btn_str}), format!("Performed {} click", btn_str)))
                },
                "type_text" => {
                    let text = params["text"].as_str().ok_or_else(|| anyhow::anyhow!("Missing 'text'"))?;
                    enigo.text(text).map_err(|e| anyhow::anyhow!("Type error: {}", e))?;
                    Ok(ToolOutput::success(json!({"length": text.len()}), format!("Typed {} characters.", text.len())))
                },
                "key_tap" => {
                    let key_name = params["key"].as_str().ok_or_else(|| anyhow::anyhow!("Missing 'key'"))?;
                    let key = match key_name.to_lowercase().as_str() {
                        "enter" | "return" => Key::Return,
                        "tab" => Key::Tab,
                        "escape" => Key::Escape,
                        "space" => Key::Space,
                        _ => return Ok(ToolOutput::failure(format!("Unsupported key: {}", key_name))),
                    };
                    enigo.key(key, Direction::Click).map_err(|e| anyhow::anyhow!("Key error: {}", e))?;
                    Ok(ToolOutput::success(json!({"key": key_name}), format!("Tapped key: {}", key_name)))
                },
                _ => Ok(ToolOutput::failure(format!("Action {} not supported by hands", action))),
            }
        }).await.map_err(|e| AgentError::Execution(e.to_string()))?;

        match result {
            Ok(out) => Ok(out),
            Err(e) => Ok(ToolOutput::failure(format!("GUI error: {}", e))),
        }
    }
}