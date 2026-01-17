//! Skill Crystallizer (Minimal)

use std::sync::Arc;
use anyhow::Result;
use tracing::{info, warn};
use serde_json::{json, Value};

use crate::agent::{LLMProvider, AgentType};
use crate::tools::{ToolRegistry, ToolOutput};
use crate::memory::Memory;

pub struct SkillCrystallizer {
    provider: Arc<dyn LLMProvider>,
    tools: Arc<ToolRegistry>,
    memory: Arc<dyn Memory>,
}

impl SkillCrystallizer {
    pub fn new(
        provider: Arc<dyn LLMProvider>,
        tools: Arc<ToolRegistry>,
        memory: Arc<dyn Memory>
    ) -> Self {
        Self { provider, tools, memory }
    }

    pub async fn crystallize(&self) -> Result<u32> {
        info!("💎 Crystallizer: Scanning for repeatable patterns...");

        // 1. Fetch recent successful task traces from Memory
        // We look for memories tagged as successful executions
        let successes = self.memory.search("successfully executed task", 10, None, None).await?;
        
        if successes.is_empty() {
            info!("💎 Crystallizer: No sufficient data to crystallize.");
            return Ok(0);
        }

        // 2. Ask LLM to identify a pattern that can be codified
        let context = successes.iter()
            .map(|m| format!("- {}", m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Analyze these recent successful tasks:\n{}\n\n\
            Identify ONE purely algorithmic or repetitive task that was solved by reasoning but COULD be solved by a simple Rust function. \
            If found, write a Rust function signature and implementation. \
            The function MUST be valid Rust, `no_mangle`, `extern \"C\"`, and perform a useful, atomic operation. \
            \
            Return JSON ONLY: {{ \"found\": true, \"name\": \"tool_name\", \"description\": \"desc\", \"code\": \"...rust code...\" }} \
            If no pattern is rigid enough for code, return {{ \"found\": false }}",
            context
        );

        let response = self.provider.generate(
            AgentType::Coder.default_model(), 
            prompt, 
            Some("You are a Compiler Architect looking for optimization opportunities.".to_string())
        ).await?;

        // 3. Parse and Compile
        let cleaned_json = response.trim().trim_matches('`').replace("json", "");
        if let Ok(plan) = serde_json::from_str::<Value>(&cleaned_json) {
            if plan["found"].as_bool().unwrap_or(false) {
                let name = plan["name"].as_str().unwrap_or("new_skill");
                let code = plan["code"].as_str().unwrap_or("");
                let description = plan["description"].as_str().unwrap_or("Dynamic skill");

                info!("💎 Crystallizer: Pattern detected! Crystallizing '{}'...", name);

                // Use the WasmCompilerTool to build it
                if let Some(compiler) = self.tools.get_tool("wasm_compiler").await {
                    let compile_res = compiler.execute(json!({
                        "filename": name,
                        "source_code": code
                    })).await?;

                    if compile_res.success {
                        info!("💎 Crystallizer: Compilation successful for '{}'.", name);
                        
                        // Save a memory about this new capability
                        self.memory.store(crate::memory::MemoryEntry::new(
                            format!("New Skill Crystallized: {}. Description: {}. Path: {}", name, description, compile_res.data["wasm_path"]),
                            "Crystallizer",
                            crate::memory::entry::MemorySource::System
                        )).await?;

                        return Ok(1);
                    } else {
                        warn!("💎 Crystallizer: Compilation failed: {}", compile_res.summary);
                    }
                }
            }
        }

        Ok(0)
    }
}
