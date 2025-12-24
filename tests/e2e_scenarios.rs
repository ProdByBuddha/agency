use anyhow::Result;
use async_trait::async_trait;
use ollama_rs::Ollama;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::VecDeque;
use tracing::debug;

use rust_agency::agent::LLMProvider;
use rust_agency::orchestrator::Supervisor;
use rust_agency::tools::ToolRegistry;
use rust_agency::memory::VectorMemory;

struct SmartMockProvider {
    responses: Arc<Mutex<VecDeque<String>>>, 
}

impl SmartMockProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
        }
    }
}

#[async_trait]
impl LLMProvider for SmartMockProvider {
    async fn generate(&self, _model: &str, prompt: String, _system: Option<String>) -> Result<String> {
        let p = prompt.to_lowercase();
        debug!("MOCK PROMPT: {}", p);
        
        // Match specific types of prompts to return appropriate mock data
        
        // 1. Router Logic
        if p.contains("classify") {
            if p.contains("joke") {
                return Ok("→ {\"agent\": \"general_chat\", \"memory\": \"no\", \"reason\": \"Simple joke request\"}".to_string());
            }
            if p.contains("search for rust") {
                return Ok("→ {\"agent\": \"planner\", \"memory\": \"yes\", \"reason\": \"Complex multi-step task\"}".to_string());
            }
        }
        
        // 2. Planner Logic
        if p.contains("decompose") {
            return Ok(r#"[
                {{\"desc\": \"Search for Rust\", \"agent\": \"researcher\", \"tools\": \"web_search\", \"expected\": \"info\"}},
                {{\"desc\": \"Save artifact\", \"agent\": \"coder\", \"tools\": \"artifact_manager\", \"expected\": \"saved\"}}
            ]"#.to_string());
        }
        
        // 3. Agent Reasoning (ReAct)
        if p.contains("save artifact") {
            if p.contains("[observation]") {
                return Ok("[THOUGHT] I have saved it. [ANSWER] Saved.".to_string());
            } else {
                return Ok("[THOUGHT] I will save. [ACTION]\n{{\"name\": \"artifact_manager\", \"parameters\": {\"action\": \"write\", \"filename\": \"r.txt\", \"content\": \"fast\"}}}".to_string());
            }
        } else if p.contains("search for rust") {
            if p.contains("[observation]") {
                return Ok("[THOUGHT] I have the info. [ANSWER] Rust is fast.".to_string());
            } else {
                return Ok("[THOUGHT] I will search. [ACTION]\n{{\"name\": \"web_search\", \"parameters\": {\"query\": \"rust\"}}}".to_string());
            }
        }
        
        // 4. Reflector Review Logic (Consensus)
        if p.contains("technical reviewer") || p.contains("analyzing a failed") {
            return Ok("ANALYSIS: Verified. SHOULD_RETRY: no".to_string());
        }
        
        // 5. Simple Chat
        if p.contains("joke") {
            return Ok("[THOUGHT] Joke... [ANSWER] Why did the Rust programmer quit? Because they didn't have enough lifetime!".to_string());
        }

        Ok("[THOUGHT] Default. [ANSWER] I'm not sure how to respond to that.".to_string())
    }
}

#[tokio::test] async fn test_e2e_routing_to_simple_chat() {
    let ollama = Ollama::default();
    let tools = Arc::new(ToolRegistry::new());
    let query = "Tell me a short joke about Rust";
    
    let provider = Arc::new(SmartMockProvider::new(vec![]));
    let mut supervisor = Supervisor::new(ollama, tools)
        .with_provider(provider);
        
    let result = supervisor.handle(query).await.unwrap();
    println!("DEBUG SIMPLE CHAT ANSWER: '{}'", result.answer);
    
    assert!(result.success);
    assert!(result.answer.contains("lifetime"));
}

#[tokio::test] async fn test_e2e_complex_planning_scenario() {
    let ollama = Ollama::default();
    let tools = Arc::new(ToolRegistry::new());
    
    let provider = Arc::new(SmartMockProvider::new(vec![]));
    
    let temp_dir = tempfile::tempdir().unwrap();
    let memory_path = temp_dir.path().join("memory.json");
    let memory = Arc::new(VectorMemory::new(memory_path).unwrap());
    
    let mut supervisor = Supervisor::new(ollama, tools)
        .with_memory(memory)
        .with_provider(provider)
        .with_max_retries(1);
        
    let result = supervisor.handle("Search for Rust and save it").await.unwrap();
    println!("DEBUG PLANNING ANSWER: '{}'", result.answer);
    
    assert!(result.success);
    assert!(result.plan.is_some());
    assert_eq!(result.answer, "Saved.");
}