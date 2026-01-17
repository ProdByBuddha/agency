//! WASM Executor Tool
//! 
//! Executes compiled WASM modules in a safe runtime.
//! Part of the Self-Correction Loop.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use crate::agent::{AgentResult, AgentError};
use crate::runtime::wasm::WasmRuntime;
use super::{Tool, ToolOutput};

pub struct WasmExecutorTool {
    runtime: Mutex<WasmRuntime>,
}

impl WasmExecutorTool {
    pub fn new() -> Self {
        Self {
            runtime: Mutex::new(WasmRuntime::new()),
        }
    }
}

#[async_trait]
impl Tool for WasmExecutorTool {
    fn name(&self) -> String {
        "wasm_executor".to_string()
    }

    fn description(&self) -> String {
        "Executes a function inside a compiled WASM module. 
        Currently supports functions signature: fn(i32, i32) -> i32.
        Use this to test and run your compiled capabilities.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "wasm_path": {
                    "type": "string",
                    "description": "Path to the .wasm file"
                },
                "function_name": {
                    "type": "string",
                    "description": "Name of the exported function to call"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "List of integer arguments (max 2)"
                }
            },
            "required": ["wasm_path", "function_name", "args"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "runtime",
            "safety": "Sandboxed (WASM)",
            "requirements": ["wasmer"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let wasm_path_str = params["wasm_path"].as_str().ok_or_else(|| AgentError::Validation("Missing wasm_path".to_string()))?;
        let function_name = params["function_name"].as_str().unwrap_or("run");
        let args_json = params["args"].as_array().ok_or_else(|| AgentError::Validation("Missing args array".to_string()))?;
        
        let mut args = Vec::new();
        for arg in args_json {
            if let Some(i) = arg.as_i64() {
                args.push(i as i32);
            }
        }

        let wasm_path = PathBuf::from(wasm_path_str);
        if !wasm_path.exists() {
            return Ok(ToolOutput::failure(format!("WASM file not found: {}", wasm_path.display())));
        }

        let result = {
            let mut runtime = self.runtime.lock().unwrap();
            runtime.execute(&wasm_path, function_name, &args)
        };

        match result {
            Ok(val) => Ok(ToolOutput::success(json!({"result": val}), format!("Execution successful. Result: {}", val))),
            Err(e) => Ok(ToolOutput::failure(format!("Runtime error: {}", e))),
        }
    }
}
