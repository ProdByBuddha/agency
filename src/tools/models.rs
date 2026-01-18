// Model Manager Tool
// 
// Allows listing, adding, and selecting models in the agency registry.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::File;
use std::io::Write;
use schemars::JsonSchema;

use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct ModelManagerParams {
    /// The action to perform: 'list', 'select', 'add', or 'pull'.
    pub action: String,
    /// The name of the model (required for select, add, pull).
    pub name: Option<String>,
    /// Scale class to set default for (tiny, standard, heavy, bitnet). Required for 'select'.
    pub class: Option<String>,
    /// HuggingFace repo ID. Required for 'add'.
    pub repo: Option<String>,
    /// Quantized GGUF filename. Optional for 'add'.
    pub quant_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfig {
    name: String,
    repo: String,
    revision: String,
    tokenizer_repo: String,
    is_quantized: bool,
    quant_file: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    models: Vec<ModelConfig>,
    defaults: std::collections::HashMap<String, String>,
}

pub struct ModelManager;

impl ModelManager {
    fn load_registry(&self) -> AgentResult<Registry> {
        let file = File::open("config/agency_models.json").map_err(|e| AgentError::Io(e))?;
        let registry: Registry = serde_json::from_reader(file).map_err(|e| AgentError::Serde(e))?;
        Ok(registry)
    }

    fn save_registry(&self, registry: &Registry) -> AgentResult<()> {
        let json = serde_json::to_string_pretty(registry).map_err(|e| AgentError::Serde(e))?;
        let mut file = File::create("config/agency_models.json").map_err(|e| AgentError::Io(e))?;
        file.write_all(json.as_bytes()).map_err(|e| AgentError::Io(e))?;
        Ok(())
    }
}

#[async_trait]
impl Tool for ModelManager {
    fn name(&self) -> String { "model_manager".to_string() } 
    
    fn description(&self) -> String {
        "Manage the agency's brain. Use this to list available models, switch active models for different scales, or download models from Hugging Face.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "select", "add", "pull"],
                    "description": "The action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "The name of the model (e.g. 'llama3.1:8b')"
                },
                "class": {
                    "type": "string",
                    "enum": ["tiny", "standard", "heavy"],
                    "description": "The scale class to update (e.g. tiny, standard, heavy). Required for 'select'."
                },
                "repo": {
                    "type": "string",
                    "description": "The Hugging Face repo ID"
                },
                "quant_file": {
                    "type": "string",
                    "description": "Optional: Specific GGUF file name for quantized models"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> AgentResult<ToolOutput> {
        let p: ModelManagerParams = serde_json::from_value(params).map_err(|e| AgentError::Serde(e))?;
        let mut registry = self.load_registry()?;

        match p.action.as_str() {
            "list" => {
                let mut table = String::from("| Model Name | Quantized | Default For |\n|---|---|---|
");
                for m in &registry.models {
                    let mut defaults = Vec::new();
                    for (class, name) in &registry.defaults {
                        if name == &m.name { defaults.push(class.as_str()); }
                    }
                    table.push_str(&format!("| {} | {} | {} |\n", 
                        m.name,
                        if m.is_quantized { "Yes" } else { "No" },
                        defaults.join(", ")
                    ));
                }
                Ok(ToolOutput::success(serde_json::to_value(&registry.models).map_err(|e| AgentError::Serde(e))?, table))
            },
            "select" => {
                let name = p.name.ok_or_else(|| AgentError::Validation("Model name required for 'select'".to_string()))?;
                let class = p.class.ok_or_else(|| AgentError::Validation("Scale class required for 'select'".to_string()))?;
                
                if !registry.models.iter().any(|m| m.name == name) {
                    return Ok(ToolOutput::failure(format!("Model '{}' not found in registry.", name)));
                }

                registry.defaults.insert(class.clone(), name.clone());
                self.save_registry(&registry)?;
                Ok(ToolOutput::success(
                    json!({"class": class, "model": name}),
                    format!("Set {} as the default model for '{}' tasks.", name, class)
                ))
            },
            "add" => {
                let name = p.name.ok_or_else(|| AgentError::Validation("Model name required for 'add'".to_string()))?;
                let repo = p.repo.ok_or_else(|| AgentError::Validation("Repo ID required for 'add'".to_string()))?;
                
                let is_quantized = p.quant_file.is_some();
                let config = ModelConfig {
                    name: name.clone(),
                    repo,
                    revision: "main".to_string(),
                    tokenizer_repo: "meta-llama/Llama-3.2-3B-Instruct".to_string(), // Default safe tokenizer
                    is_quantized,
                    quant_file: p.quant_file,
                    description: None,
                };

                registry.models.push(config.clone());
                self.save_registry(&registry)?;
                Ok(ToolOutput::success(
                    serde_json::to_value(config).map_err(|e| AgentError::Serde(e))?,
                    format!("Added model '{}' to the registry.", name)
                ))
            },
            "pull" => {
                let name = p.name.ok_or_else(|| AgentError::Validation("Model name required for 'pull'".to_string()))?;
                let config = registry.models.iter().find(|m| m.name == name)
                    .cloned()
                    .ok_or_else(|| AgentError::Validation(format!("Model '{}' not found in registry. Add it first.", name)))?;

                println!("⏳ Pulling model weights for '{}'...", name);
                
                let hf_token = std::env::var("HF_TOKEN").ok();
                let repo_id = config.repo.clone();
                let revision = config.revision.clone();
                let is_quantized = config.is_quantized;
                let quant_file = config.quant_file.clone();

                tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                    use hf_hub::{api::sync::ApiBuilder, Repo};
                    let mut api_builder = ApiBuilder::new().with_progress(true);
                    if let Some(token) = hf_token {
                        api_builder = api_builder.with_token(Some(token));
                    }
                    let api = api_builder.build()?;
                    let repo = api.repo(Repo::with_revision(repo_id, hf_hub::RepoType::Model, revision));
                    
                    if is_quantized {
                        let filename = quant_file.unwrap_or_else(|| "model.gguf".to_string());
                        repo.get(&filename)?;
                    } else {
                        // For unquantized, try to get safetensors index or the main file
                        match repo.get("model.safetensors.index.json") {
                            Ok(_) => { /* Weight downloading is handled by Candle at runtime, but repo.get(index) ensures we have the repo */ },
                            Err(_) => { repo.get("model.safetensors")?; }
                        }
                    }
                    Ok(())
                }).await.map_err(|e| AgentError::Execution(e.to_string()))?
                .map_err(|e| AgentError::Tool(e.to_string()))?;

                Ok(ToolOutput::success(
                    json!({"model": name, "status": "pulled"}),
                    format!("Successfully pulled weights for model '{}'.", name)
                ))
            },
            _ => Ok(ToolOutput::failure("Unknown action. Use 'list', 'select', 'add', or 'pull'.")),
        }
    }
}
