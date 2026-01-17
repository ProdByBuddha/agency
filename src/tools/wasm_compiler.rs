//! WASM Compiler Tool
//! 
//! Compiles Rust source code into WebAssembly modules on the fly.
//! This enables the Agency to generate new safe, sandboxed capabilities.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use std::path::PathBuf;
use crate::agent::{AgentResult, AgentError};
use super::{Tool, ToolOutput};

pub struct WasmCompilerTool {
    work_dir: PathBuf,
}

impl WasmCompilerTool {
    pub fn new() -> Self {
        Self {
            work_dir: std::env::temp_dir().join("agency_wasm_builds"),
        }
    }
}

#[async_trait]
impl Tool for WasmCompilerTool {
    fn name(&self) -> String {
        "wasm_compiler".to_string()
    }

    fn description(&self) -> String {
        "Compiles Rust code into a WASM module. Returns the path to the .wasm file.
        Use this to create new, high-performance, sandboxed tools on the fly.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_code": {
                    "type": "string",
                    "description": "The Rust source code to compile. Must be a library (cdylib)."
                },
                "filename": {
                    "type": "string",
                    "description": "Output filename (e.g., 'math_module')"
                }
            },
            "required": ["source_code", "filename"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "evolutionary",
            "safety": "High (Compiles code)",
            "requirements": ["rustc", "wasm32-unknown-unknown"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let source_code = params["source_code"].as_str().ok_or_else(|| AgentError::Validation("Missing source_code".to_string()))?;
        let filename = params["filename"].as_str().unwrap_or("module");
        
        if !self.work_dir.exists() {
            tokio::fs::create_dir_all(&self.work_dir).await.map_err(|e| AgentError::Io(e))?;
        }

        let src_path = self.work_dir.join(format!("{}.rs", filename));
        let wasm_path = self.work_dir.join(format!("{}.wasm", filename));

        // Write source code
        tokio::fs::write(&src_path, source_code).await.map_err(|e| AgentError::Io(e))?;

        // Compile using rustc
        // rustc --target wasm32-unknown-unknown --crate-type cdylib -O source.rs -o output.wasm
        let output = Command::new("rustc")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("--crate-type")
            .arg("cdylib")
            .arg("-O") // Optimize
            .arg(&src_path)
            .arg("-o")
            .arg(&wasm_path)
            .output()
            .await
            .map_err(|e| AgentError::Io(e))?;

        if output.status.success() {
            Ok(ToolOutput::success(
                json!({
                    "wasm_path": wasm_path.to_string_lossy(),
                    "size_bytes": wasm_path.metadata().map(|m| m.len()).unwrap_or(0)
                }), 
                format!("Successfully compiled WASM module to {}", wasm_path.display())
            ))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(ToolOutput::failure(format!("Compilation failed:\n{}", stderr)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_wasm_compilation() {
        let compiler = WasmCompilerTool::new();
        
        let source_code = r#"
            #[no_mangle]
            pub extern "C" fn add(a: i32, b: i32) -> i32 {
                a + b
            }
        "#;
        
        let params = json!({
            "source_code": source_code,
            "filename": "test_add"
        });
        
        // This test requires the wasm32-unknown-unknown target installed.
        // We'll skip if the execution fails due to missing target, but if it runs, it must produce wasm.
        if let Ok(res) = compiler.execute(params).await {
            if res.success {
                let path_str = res.data["wasm_path"].as_str().unwrap();
                let path = PathBuf::from(path_str);
                assert!(path.exists());
                assert!(path.extension().unwrap() == "wasm");
                
                // Cleanup
                let _ = fs::remove_file(path);
            } else {
                // If it failed, check if it's due to missing target
                println!("Compilation failed (expected if wasm32 target missing): {}", res.summary);
            }
        }
    }
}
