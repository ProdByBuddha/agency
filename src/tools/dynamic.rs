//! Dynamic Tool Implementation
//! 
//! Allows for loading and executing custom scripts as first-class tools.

use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{debug, warn};

use crate::agent::{AgentResult, AgentError};
use super::{Tool, ToolOutput, ToolRegistry};

/// Metadata for a dynamic tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolMetadata {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub language: String, // "python", "shell", "node"
    pub script_path: String,
}

/// A tool that executes an external script
pub struct DynamicTool {
    metadata: DynamicToolMetadata,
    base_path: PathBuf,
}

impl DynamicTool {
    pub fn new(metadata: DynamicToolMetadata, base_path: PathBuf) -> Self {
        Self { metadata, base_path }
    }

    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read tool metadata at {:?}", path))?;
        let metadata: DynamicToolMetadata = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse tool metadata at {:?}", path))?;
        
        let base_path = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        Ok(Self { metadata, base_path })
    }
}

#[async_trait]
impl Tool for DynamicTool {
    fn name(&self) -> String {
        self.metadata.name.clone()
    }

    fn description(&self) -> String {
        self.metadata.description.clone()
    }

    fn parameters(&self) -> Value {
        self.metadata.parameters.clone()
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "custom",
            "notes": "WorkScope depends on the dynamically loaded tool implementation."
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let script_abs_path = self.base_path.join(&self.metadata.script_path);
        
        if !script_abs_path.exists() {
            return Ok(ToolOutput::failure(format!("Script not found: {:?}", self.metadata.script_path)));
        }

        let params_json = serde_json::to_string(&params)?;
        
        let script_str = script_abs_path.to_str().ok_or_else(|| AgentError::Validation("Invalid script path".to_string()))?;

        let (cmd, args) = match self.metadata.language.as_str() {
            "python" => ("python3".to_string(), vec![script_str.to_string(), params_json]),
            "node" => ("node".to_string(), vec![script_str.to_string(), params_json]),
            "shell" => ("sh".to_string(), vec![script_str.to_string(), params_json]),
            "rust" => {
                // For Rust, we compile to a binary first
                let binary_path = script_abs_path.with_extension("");
                let binary_str = binary_path.to_str().ok_or_else(|| AgentError::Validation("Invalid binary path".to_string()))?;
                
                let compile_status = Command::new("rustc")
                    .arg(script_str)
                    .arg("-o")
                    .arg(binary_str)
                    .status()
                    .await
                    .map_err(|e| AgentError::Tool(format!("Failed to spawn rustc: {}", e)))?;

                if !compile_status.success() {
                    return Ok(ToolOutput::failure("Failed to compile dynamic Rust tool"));
                }
                (binary_str.to_string(), vec![params_json])
            },
            _ => return Ok(ToolOutput::failure(format!("Unsupported language: {}", self.metadata.language))),
        };

        debug!("Executing dynamic tool {} using {}", self.metadata.name, cmd);

        let result = timeout(
            Duration::from_secs(60),
            Command::new(&cmd)
                .args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .output()
        ).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                if exit_code == 0 {
                    Ok(ToolOutput::success(
                        json!({ "stdout": stdout, "stderr": stderr }),
                        stdout
                    ))
                } else {
                    Ok(ToolOutput {
                        success: false,
                        data: json!({ "stdout": stdout, "stderr": stderr, "exit_code": exit_code }),
                        summary: format!("Tool failed with exit code {}.\nError: {}", exit_code, stderr),
                        error: Some(stderr),
                    })
                }
            }
            Ok(Err(e)) => Err(AgentError::Tool(format!("Failed to execute dynamic tool: {}", e))),
            Err(_) => Err(AgentError::Execution("Dynamic tool execution timed out".to_string())),
        }
    }
}

/// Tool for forging new tools
pub struct ForgeTool {
    custom_tools_dir: PathBuf,
    registry: Arc<ToolRegistry>,
}

impl ForgeTool {
    pub fn new(dir: impl Into<PathBuf>, registry: Arc<ToolRegistry>) -> Self {
        Self { 
            custom_tools_dir: dir.into(),
            registry,
        }
    }
}

#[async_trait]
impl Tool for ForgeTool {
    fn name(&self) -> String {
        "forge_tool".to_string()
    }

    fn description(&self) -> String {
        "Forge a new specialized tool by providing metadata and a script.\n
         The new tool will be permanently available to the agency and CAN BE USED IMMEDIATELY in the next step.\n 
         BY DEFAULT, tools should be forged in 'rust' unless specifically requested otherwise by the human or necessitated by complex logic.\n 
         Use this when you need a specialized functionality that doesn't exist yet (e.g. specialized file parsing, data transformation, or API interaction).".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Unique name for the tool (snake_case)" },
                "description": { "type": "string", "description": "What the tool does" },
                "parameters": { "type": "object", "description": "JSON schema for tool parameters" },
                "language": { "type": "string", "enum": ["python", "shell", "node", "rust"], "default": "rust" },
                "code": { "type": "string", "description": "The actual script code" }
            },
            "required": ["name", "description", "parameters", "language", "code"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "highly_agential",
            "capability": "metaprogramming (self-expansion)",
            "safety": "high risk: requires review of forged logic",
            "persistence": "forged tools are saved to disk"
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let name = params["name"].as_str().ok_or_else(|| AgentError::Validation("Missing name".to_string()))?;
        let description = params["description"].as_str().ok_or_else(|| AgentError::Validation("Missing description".to_string()))?;
        let language = params["language"].as_str().ok_or_else(|| AgentError::Validation("Missing language".to_string()))?;
        let code = params["code"].as_str().ok_or_else(|| AgentError::Validation("Missing code".to_string()))?;
        
        let (ext, is_safe) = match language {
            "python" => ("py", true),
            "node" => ("js", true),
            "shell" => ("sh", false), // Shell is higher risk
            "rust" => ("rs", true),
            _ => ("script", false),
        };

        if !is_safe {
            warn!("Forging high-risk tool: {} in {}", name, language);
        }

        let script_filename = format!("{}.{}", name, ext);
        let metadata_filename = format!("{}.json", name);
        
        let script_path = self.custom_tools_dir.join(&script_filename);
        let metadata_path = self.custom_tools_dir.join(&metadata_filename);

        // Ensure directory exists
        if !self.custom_tools_dir.exists() {
            std::fs::create_dir_all(&self.custom_tools_dir)?;
        }

        // Write script
        std::fs::write(&script_path, code)?;
        
        // Write metadata
        let metadata = DynamicToolMetadata {
            name: name.to_string(),
            description: description.to_string(),
            parameters: params["parameters"].clone(),
            language: language.to_string(),
            script_path: script_filename,
        };
        
        std::fs::write(&metadata_path, serde_json::to_string_pretty(&metadata)?)?;

        // IMMEDIATE HOT-RELOAD: Register the new tool in the active registry
        let new_tool = DynamicTool::new(metadata, self.custom_tools_dir.clone());
        self.registry.register_instance(new_tool).await;

        Ok(ToolOutput::success(
            json!({ "status": "success", "tool": name }),
            format!("Successfully forged tool '{}'. It is now loaded and available for immediate use.", name)
        ))
    }

    fn requires_confirmation(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_forge_tool_execute() {
        let registry = Arc::new(ToolRegistry::default());
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let tool = ForgeTool::new(temp_dir.path(), registry.clone());
        
        let params = json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {"type": "object"},
            "language": "python",
            "code": "print('hello')"
        });
        
        let res = tool.execute(params).await.expect("Tool execution failed");
        assert!(res.success);
        
        // Check if files were created
        assert!(temp_dir.path().join("test_tool.py").exists());
        assert!(temp_dir.path().join("test_tool.json").exists());
        
        // Check if hot-reloaded into registry
        let tool_names = registry.tool_names().await;
        assert!(tool_names.contains(&"test_tool".to_string()));
    }
}
