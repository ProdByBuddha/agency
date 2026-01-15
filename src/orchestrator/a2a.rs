//! A2A (Agent-to-Agent) Communication
//! 
//! Implements the protocol for direct collaboration between agents.
//! Aligns with FPF U.Interaction (A.1) and uses SNS for token efficiency.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use chrono::Utc;
use uuid::Uuid;

use crate::agent::{AgentType, AgentResponse, AgentResult};
use crate::orchestrator::Supervisor;

/// FPF-aligned Agent Interaction (A.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInteraction {
    pub interaction_id: String,
    pub source_agent: AgentType,
    pub target_agent: AgentType,
    /// The message content, preferably in SNS notation
    pub payload: String,
    pub timestamp: chrono::DateTime<Utc>,
    /// Evidence context passed from the requester
    pub trace_context: Vec<String>,
}

impl AgentInteraction {
    pub fn new(source: AgentType, target: AgentType, payload: impl Into<String>) -> Self {
        Self {
            interaction_id: Uuid::new_v4().to_string(),
            source_agent: source,
            target_agent: target,
            payload: payload.into(),
            timestamp: Utc::now(),
            trace_context: Vec::new(),
        }
    }
}

/// The A2A Bridge facilitates direct peer-to-peer calls
pub struct A2ABridge {
    supervisor: Arc<Mutex<Supervisor>>,
}

impl A2ABridge {
    pub fn new(supervisor: Arc<Mutex<Supervisor>>) -> Self {
        Self { supervisor }
    }

    /// Execute a peer call between two agents
    pub async fn peer_call(&self, interaction: AgentInteraction) -> AgentResult<AgentResponse> {
        let mut supervisor = self.supervisor.lock().await;
        
                // 1. Prepare A2A-specific context
        
                let mut a2a_context = format!(
        
                    "\n<|im_start|>system\nDIRECT PEER CALL [ID: {}]\nSOURCE: {:?}\nTARGET: {:?}\n",
        
                    interaction.interaction_id, interaction.source_agent, interaction.target_agent
        
                );
        
                
        
                a2a_context.push_str("INSTRUCTION: You are being consulted as a peer. Use SNS for the response if possible.\n");
        
                
        
                if !interaction.trace_context.is_empty() {
        
                    a2a_context.push_str("RELEVANT TRACE:\n");
        
                    for trace in interaction.trace_context {
        
                        a2a_context.push_str(&format!("- {}\n", trace));
        
                    }
        
                }
        
                a2a_context.push_str("<|im_end|>\n");
        
        
        
                // 2. Delegate to Supervisor's peer handling
        
                supervisor.handle_peer_request(
        
                    interaction.target_agent,
        
                    &interaction.payload,
        
                    Some(&a2a_context)
        
                ).await
        
            }
        
        }
        
        