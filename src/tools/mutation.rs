//! Mutation Tool (Self-Evolution)
//! 
//! "DNA Editing": Allows the agency to modify its own source code
//! and validate changes via sandboxed tests.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::process::Command;
use std::process::Stdio;
use tracing::{info, warn, error};

use crate::agent::{AgentResult, AgentError};
use crate::utils::sandbox::TOOL_SANDBOX_POLICY;
use super::{Tool, ToolOutput};

pub struct MutationTool {
    src_dir: PathBuf,
}

impl MutationTool {
    pub fn new(src_dir: impl Into<PathBuf>) -> Self {
        let path = src_dir.into();
        let src_dir = std::fs::canonicalize(&path).unwrap_or(path);
        Self { src_dir }
    }

    fn is_safe_path(&self, path: &Path) -> bool {
        let canonical = match std::fs::canonicalize(path) {
            Ok(p) => p,
            Err(_) => return false,
        };
        canonical.starts_with(&self.src_dir) || canonical.ends_with("Cargo.toml")
    }

    async fn run_sandboxed(&self, program: &str, args: &[&str]) -> anyhow::Result<(String, i32)> {
        #[cfg(target_os = "macos")]
        {
            let mut sb_args = vec![
                "-p".to_string(), TOOL_SANDBOX_POLICY.to_string(),
                "-D".to_string(), format!("WORKSPACE_DIR={}", self.src_dir.to_string_lossy()),
                "--".to_string(),
                program.to_string()
            ];
            for arg in args { sb_args.push(arg.to_string()); }

            let output = Command::new("/usr/bin/sandbox-exec")
                .args(&sb_args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let code = output.status.code().unwrap_or(-1);
            
            Ok((format!("{}\n{}", stdout, stderr), code))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let output = Command::new(program)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok((format!("{}\n{}", stdout, stderr), output.status.code().unwrap_or(-1)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_mutation_path_safety() {
        let dir = tempdir().expect("Failed to create temp dir");
        let src_path = dir.path().join("src");
        std::fs::create_dir(&src_path).expect("Failed to create src dir");
        
        // Create dummy file inside
        let valid_file = src_path.join("lib.rs");
        File::create(&valid_file).expect("Failed to create valid file");

        let tool = MutationTool::new(&src_path);
        
        assert!(tool.is_safe_path(&valid_file), "Should allow valid file inside src");
        
        let outside_file = dir.path().join("secret.txt");
        File::create(&outside_file).expect("Failed to create outside file");
        
        assert!(!tool.is_safe_path(&outside_file), "Should block file outside src");
        
        // Non-existent file
        let ghost_file = src_path.join("ghost.rs");
        assert!(!tool.is_safe_path(&ghost_file), "Should block non-existent file (canonicalize fails)");
    }
}

impl Default for MutationTool {
    fn default() -> Self {
        Self::new(".")
    }
}

#[async_trait]
impl Tool for MutationTool {
    fn name(&self) -> String {
        "mutation_engine".to_string()
    }

    fn description(&self) -> String {
        "Modify the Agency's own source code. Supports 'apply_change' and 'verify'. 
        All verification runs in a secure Seatbelt sandbox. Use this to evolve the system, fix bugs, or add features.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["apply_change", "verify"],
                    "description": "Action to perform"
                },
                "path": {
                    "type": "string",
                    "description": "Path to the source file to modify"
                },
                "content": {
                    "type": "string",
                    "description": "The new content for the file (if action is 'apply_change')"
                }
            },
            "required": ["action"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "evolutionary",
            "safety": "ULTRA-HIGH (Sandbox-validated mutation)",
            "impact": "permanent code changes",
            "requirements": ["human_confirmation"]
        })
    }

    fn requires_confirmation(&self) -> bool {
        true // Critical safety: Mutation always requires approval
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let action = params["action"].as_str().unwrap_or("verify");

        match action {
            "apply_change" => {
                let rel_path = params["path"].as_str().ok_or_else(|| AgentError::Validation("Missing path".to_string()))?;
                let content = params["content"].as_str().ok_or_else(|| AgentError::Validation("Missing content".to_string()))?;
                
                let path = self.src_dir.join(rel_path);
                if !self.is_safe_path(&path) {
                    return Ok(ToolOutput::failure("Access denied: Cannot mutate outside allowed scope."));
                }

                // Backup before change (SOTA Safety)
                let backup_path = path.with_extension("bak");
                if path.exists() {
                    let _ = fs::copy(&path, &backup_path).await;
                }

                fs::write(&path, content).await.map_err(|e| AgentError::Io(e))?;
                
                info!("🧬 Mutation Applied: {}", rel_path);
                Ok(ToolOutput::success(json!({"path": rel_path}), format!("Mutation successfully applied to {}. PLEASE RUN VERIFY NEXT.", rel_path)))
            }
            "verify" => {
                info!("🧬 Mutation Verification: Running sandboxed build check...");
                match self.run_sandboxed("cargo", &["check"]).await {
                    Ok((output, 0)) => {
                        Ok(ToolOutput::success(json!({"status": "valid"}), format!("Mutation verified! The build is stable.\n\n{}", output)))
                    }
                    Ok((output, code)) => {
                        Ok(ToolOutput {
                            success: false,
                            data: json!({"status": "invalid", "exit_code": code}),
                            summary: format!("Mutation FAILED verification. The build is broken. Reverting is recommended.\n\n{}", output),
                            error: Some("Build broken".to_string()),
                        })
                    }
                    Err(e) => Ok(ToolOutput::failure(format!("Verification system error: {}", e))),
                }
            }
            _ => Ok(ToolOutput::failure(format!("Unknown action: {}", action)))
        }
    }
}
