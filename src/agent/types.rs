use serde::{Deserialize, Serialize};
use crate::orchestrator::profile::AgencyProfile;

/// Types of specialized agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    GeneralChat,
    Reasoner,
    Coder,
    Researcher,
    Planner,
    Reviewer,
}

impl AgentType {
    /// Get the default model for this agent type
    pub fn default_model(&self) -> &'static str {
        match self {
            AgentType::GeneralChat => "glm-4-flash",
            AgentType::Reasoner => "glm-4",
            AgentType::Coder => "glm-4",
            AgentType::Researcher => "glm-4-flash",
            AgentType::Planner => "glm-4",
            AgentType::Reviewer => "glm-4",
        }
    }

    /// Generate a system prompt based on agent type and agency profile
    pub fn generate_system_prompt(&self, profile: &AgencyProfile) -> String {
        let base = match self {
            AgentType::GeneralChat => 
                format!("You are '{}', a high-fidelity intelligence layer. \
                 Follow the First Principles Framework (FPF): ALWAYS separate internal thought from external communication. \
                 You have access to a variety of specialized tools; use them whenever necessary to provide accurate and grounded information. \
                 Answer directly and concisely. IGNORE irrelevant conversational artifacts.", profile.name),
            
            AgentType::Reasoner => 
                "You are a logical reasoning assistant (ReasonerRole). \
                 Use [PLANNING] to determine strategy, [REASONING] for step-by-step logic, and [ANSWER] for your final projection. \
                 Verify all claims against current evidence (U.Episteme).".to_string(),
            
            AgentType::Coder => 
                "You are an expert programmer (CoderRole). \
                 Use 'codebase_explorer' to ground your claims in the physical source code. \
                 Adhere to the Strict Distinction: do not assume code state without explicit observation.".to_string(),
            
            AgentType::Researcher => 
                "You are a research assistant (ResearcherRole). \
                 Formulate search queries to build an auditable Evidence Graph. \
                 Synthesize findings using the Multi-View Publication Kit (MVPK) principles.".to_string(),
            
            AgentType::Planner => 
                "You are a task decomposition specialist (PlannerRole). \
                 Break goals into discrete MethodDescriptions. Maintain the Design-Run separation in your execution plans.".to_string(),

            AgentType::Reviewer =>
                "You are a technical reviewer (ReviewerRole). \
                 Calculate the Reliability (R) of answers based on the evidence trace. \
                 Detect and penalize epistemic drift or hallucination.".to_string(),
        };

        format!("{}\n\nAGENCY CONTEXT (U.BoundedContext):\n- Name: {}\n- Mission: {}\n- Traits: {}", 
            base, profile.name, profile.mission, profile.traits.join(", "))
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentType::GeneralChat => write!(f, "GeneralChat"),
            AgentType::Reasoner => write!(f, "Reasoner"),
            AgentType::Coder => write!(f, "Coder"),
            AgentType::Researcher => write!(f, "Researcher"),
            AgentType::Planner => write!(f, "Planner"),
            AgentType::Reviewer => write!(f, "Reviewer"),
        }
    }
}

/// Configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_type: AgentType,
    pub model: String,
    pub system_prompt: String,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    /// Which tools this agent can use
    pub allowed_tools: Vec<String>,
    /// Which tools are in the 'laboratory' (not yet promoted)
    pub laboratory_tools: Vec<String>,
    /// Max iterations for ReAct loop
    pub max_iterations: usize,
    /// Optional URL for OpenAI-compatible provider (e.g. vLLM)
    pub provider_url: Option<String>,
    /// Whether to enforce strict reasoning/planning tags
    pub reasoning_enabled: bool,
}

impl AgentConfig {
    pub fn new(agent_type: AgentType, profile: &AgencyProfile) -> Self {
        let allowed_tools = match agent_type {
            AgentType::GeneralChat => vec![
                "agency_control".to_string(), 
                "speaker_rust".to_string(),
                "codebase_explorer".to_string(),
                "artifact_manager".to_string(),
                "memory_query".to_string(),
                "knowledge_graph_viewer".to_string(),
                "system_monitor".to_string(),
                "web_search".to_string(),
                "code_exec".to_string(),
                "sandbox".to_string(),
                "model_manager".to_string(),
                "visualization_tool".to_string(),
                "science_tool".to_string(),
                "forge_tool".to_string()
            ],
            AgentType::Reasoner => vec![
                "agency_control".to_string(), 
                "speaker_rust".to_string(),
                "codebase_explorer".to_string(),
                "artifact_manager".to_string(),
                "memory_query".to_string(),
                "knowledge_graph_viewer".to_string(),
                "system_monitor".to_string(),
                "web_search".to_string(),
                "code_exec".to_string(),
                "sandbox".to_string(),
                "model_manager".to_string(),
                "visualization_tool".to_string(),
                "science_tool".to_string(),
                "forge_tool".to_string()
            ],
            AgentType::Coder => vec![
                "codebase_explorer".to_string(), 
                "code_exec".to_string(), 
                "sandbox".to_string(), 
                "artifact_manager".to_string(), 
                "forge_tool".to_string(), 
                "system_monitor".to_string(),
                "agency_control".to_string(),
                "speaker_rust".to_string(),
                "web_search".to_string(),
                "model_manager".to_string(),
                "visualization_tool".to_string()
            ],
            AgentType::Researcher => vec![
                "web_search".to_string(), 
                "memory_query".to_string(), 
                "speaker_rust".to_string(),
                "codebase_explorer".to_string(),
                "artifact_manager".to_string(),
                "knowledge_graph_viewer".to_string()
            ],
            AgentType::Planner => vec!["speaker_rust".to_string()],
            AgentType::Reviewer => vec!["speaker_rust".to_string()],
        };

        Self {
            agent_type,
            model: agent_type.default_model().to_string(),
            system_prompt: agent_type.generate_system_prompt(profile),
            temperature: 0.7,
            max_tokens: None,
            allowed_tools,
            laboratory_tools: Vec::new(),
            max_iterations: 5,
            provider_url: None,
            reasoning_enabled: true,
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self::new(AgentType::Reasoner, &AgencyProfile::default())
    }
}

