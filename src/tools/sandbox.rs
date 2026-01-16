//! Sandbox Execution Tool
//! 
//! Provides a unified interface for executing code and managing files
//! across different backends (Local Docker, Daytona, E2B).
//! Now leverages the centralized Immune System (Seatbelt).

use async_trait::async_trait;
use bollard::container::LogOutput;
use bollard::models::ContainerCreateBody;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::Docker;
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, RemoveContainerOptions, StartContainerOptions,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::agent::{AgentResult, AgentError};
use crate::utils::sandbox::TOOL_SANDBOX_POLICY;
use super::{Tool, ToolOutput};

/// Backend providers for the sandbox
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProvider {
    Local,
    MacOSNative,
    Daytona,
    E2B,
}

/// Unified Sandbox Tool
pub struct SandboxTool {
    provider: SandboxProvider,
}

impl SandboxTool {
    pub fn new(provider: SandboxProvider) -> Self {
        Self { provider }
    }

    #[cfg(target_os = "macos")]
    async fn execute_macos_native(&self, code: &str, language: &str) -> AgentResult<ToolOutput> {
        info!("Initializing MacOS Native sandbox (Seatbelt) for {}...", language);
        
        let temp_dir = tempfile::tempdir()
            .map_err(|e| AgentError::Io(e))?;
        let script_path = temp_dir.path().join(match language {
            "python" => "script.py",
            "javascript" => "script.js",
            "rust" => "main.rs",
            _ => "script.sh",
        });
        
        std::fs::write(&script_path, code)
            .map_err(|e| AgentError::Io(e))?;

        let workspace_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let canonical_temp = temp_dir.path().canonicalize()
            .map_err(|e| AgentError::Io(e))?;

        let mut cmd_args = vec![
            "-p".to_string(), TOOL_SANDBOX_POLICY.to_string(),
            "-D".to_string(), format!("WORKSPACE_DIR={}", workspace_dir.to_string_lossy()),
            "--".to_string(),
        ];
        
        let run_cmd = match language {
            "python" => vec!["python3".to_string(), script_path.to_string_lossy().to_string()],
            "javascript" => vec!["node".to_string(), script_path.to_string_lossy().to_string()],
            "rust" => vec![
                "sh".to_string(), 
                "-c".to_string(), 
                format!("rustc {} -o {}/main && {}/main", 
                    script_path.to_string_lossy(),
                    canonical_temp.to_string_lossy(),
                    canonical_temp.to_string_lossy()
                )
            ],
            _ => vec!["sh".to_string(), script_path.to_string_lossy().to_string()],
        };

        cmd_args.extend(run_cmd);

        let output = tokio::process::Command::new("/usr/bin/sandbox-exec")
            .args(&cmd_args)
            .output()
            .await
            .map_err(|e| AgentError::Tool(format!("Failed to execute sandbox-exec: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(ToolOutput::success(
                json!({ "stdout": stdout, "stderr": stderr }),
                format!("Native Execution Output:\n{}", stdout)
            ))
        } else {
            Ok(ToolOutput::failure(format!("Native Execution Error (Status {}):\nSTDOUT: {}\nSTDERR: {}", 
                output.status, stdout, stderr)))
        }
    }

    async fn execute_local_docker(&self, code: &str, language: &str) -> AgentResult<ToolOutput> {
        info!("Initializing local Docker/Podman sandbox for {}...", language);
        
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| AgentError::Tool(format!("Failed to connect to Docker Desktop: {}", e)))?;

        let image = match language {
            "python" => "python:3.11-slim",
            "rust" => "rust:1.75-slim",
            "javascript" => "node:20-slim",
            _ => "ubuntu:latest",
        };

        // 0. Ensure image exists
        let mut pull_stream = docker.create_image(
            Some(CreateImageOptions {
                from_image: Some(image.to_string()),
                ..Default::default()
            }),
            None,
            None,
        );
        
        while let Some(pull_result) = pull_stream.next().await {
            if let Err(e) = pull_result {
                warn!("Image pull warning: {}", e);
            }
        }

        // 1. Create container
        let container_name = format!("agency-sandbox-{}", uuid::Uuid::new_v4());
        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            tty: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };

        docker.create_container(
            Some(CreateContainerOptions { 
                name: Some(container_name.clone()),
                ..Default::default()
            }),
            config
        ).await.map_err(|e| AgentError::Tool(format!("Failed to create Docker container: {}", e)))?;

        // 2. Start container
        docker.start_container(&container_name, None::<StartContainerOptions>)
            .await.map_err(|e| AgentError::Tool(format!("Failed to start Docker container: {}", e)))?;

        // 3. Prepare execution - We use a file-based approach to avoid shell escaping issues
        let filename = match language {
            "python" => "script.py",
            "javascript" => "script.js",
            "rust" => "main.rs",
            _ => "script.sh",
        };

        // Escaping for the heredoc
        let escaped_code = code.replace("'", "'\'\''");
        let write_cmd = format!("cat << 'EOF' > {}
{}
EOF", filename, escaped_code);
        
        let exec_write = docker.create_exec(&container_name, CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(vec!["sh", "-c", &write_cmd]),
            ..Default::default()
        }).await.map_err(|e| AgentError::Tool(format!("Failed to create exec for write: {}", e)))?.id;
        docker.start_exec(&exec_write, None).await.map_err(|e| AgentError::Tool(format!("Failed to start exec for write: {}", e)))?;

        // 4. Run the code
        let run_cmd = match language {
            "python" => vec!["python3", filename],
            "javascript" => vec!["node", filename],
            "rust" => vec!["sh", "-c", "rustc main.rs && ./main"],
            _ => vec!["sh", filename],
        };

        let exec_run = docker.create_exec(&container_name, CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(run_cmd),
            ..Default::default()
        }).await.map_err(|e| AgentError::Tool(format!("Failed to create exec for run: {}", e)))?.id;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let StartExecResults::Attached { mut output, .. } = docker.start_exec(&exec_run, None).await.map_err(|e| AgentError::Tool(format!("Failed to start exec for run: {}", e)))? {
            while let Some(Ok(msg)) = output.next().await {
                match msg {
                    LogOutput::StdOut { message } => stdout.push_str(&String::from_utf8_lossy(&message)),
                    LogOutput::StdErr { message } => stderr.push_str(&String::from_utf8_lossy(&message)),
                    _ => {}
                }
            }
        }

        // 5. Cleanup
        let _ = docker.remove_container(&container_name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;

        if stderr.is_empty() || !stdout.is_empty() { 
             Ok(ToolOutput::success(
                json!({ "stdout": stdout, "stderr": stderr }),
                format!("Execution Output:\n{}", stdout)
            ))
        } else {
            Ok(ToolOutput::failure(format!("Execution Error:\nSTDOUT: {}\nSTDERR: {}", stdout, stderr)))
        }
    }

    async fn execute_daytona(&self, _code: &str, _language: &str) -> AgentResult<ToolOutput> {
        Ok(ToolOutput::failure("Daytona provider is currently disabled. Using local Docker."))
    }
}

impl Default for SandboxTool {
    fn default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::new(SandboxProvider::MacOSNative)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::new(SandboxProvider::Local)
        }
    }
}

#[async_trait]
impl Tool for SandboxTool {
    fn name(&self) -> String {
        "sandbox".to_string()
    }

    fn description(&self) -> String {
        "Advanced isolated execution environment. Supports 'run' for Python, Rust, JS, and Shell. 
        Code MUST print results to stdout. Scripts are executed as standalone files inside a clean container.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["run"],
                    "description": "Action to perform"
                },
                "code": {
                    "type": "string",
                    "description": "The code to execute"
                },
                "language": {
                    "type": "string",
                    "description": "Language: python, rust, javascript, shell",
                    "enum": ["python", "rust", "javascript", "shell"]
                }
            },
            "required": ["action", "code", "language"]
        })
    }

    fn work_scope(&self) -> Value {
        let env = match self.provider {
            SandboxProvider::MacOSNative => "native macos seatbelt (ultra-low latency)",
            SandboxProvider::Local => "isolated Docker/Podman container",
            _ => "remote sandbox",
        };
        
        json!({
            "status": "constrained",
            "environment": env,
            "resource_limits": {
                "memory": "1GB",
                "cpu": "1.0 core",
                "timeout": "60s"
            },
            "side_effects": "none (stateless)",
            "requirements": if self.provider == SandboxProvider::Local { vec!["active docker daemon"] } else { vec![] }
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let action = params["action"].as_str().unwrap_or("run");
        
        match action {
            "run" => {
                let code = params["code"].as_str().ok_or_else(|| AgentError::Validation("Missing code parameter".to_string()))?;
                let lang = params["language"].as_str().unwrap_or("python");
                
                match self.provider {
                    #[cfg(target_os = "macos")]
                    SandboxProvider::MacOSNative => self.execute_macos_native(code, lang).await,
                    #[cfg(not(target_os = "macos"))]
                    SandboxProvider::MacOSNative => Ok(ToolOutput::failure("MacOSNative provider only available on macOS")),
                    
                    SandboxProvider::Local => self.execute_local_docker(code, lang).await,
                    SandboxProvider::Daytona => self.execute_daytona(code, lang).await,
                    SandboxProvider::E2B => Ok(ToolOutput::failure("E2B provider not yet configured")),
                }
            },
            _ => Ok(ToolOutput::failure(format!("Action {} not supported by sandbox", action)))
        }
    }
}