//! A2A (Agent-to-Agent) Tool
//! 
//! Exposes other agents as callable tools.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::agent::{AgentResult, AgentError, AgentType, AgentResponse};
use crate::orchestrator::a2a::{AgentInteraction, A2ABridge};
use crate::orchestrator::Supervisor;
use super::{Tool, ToolOutput};

pub struct PeerAgentTool {
    target_agent: AgentType,
    bridge: A2ABridge,
}

impl PeerAgentTool {
    pub fn new(target: AgentType, supervisor: Arc<Mutex<Supervisor>>) -> Self {
        Self {
            target_agent: target,
            bridge: A2ABridge::new(supervisor),
        }
    }
}

#[async_trait]
impl Tool for PeerAgentTool {
    fn name(&self) -> String {
        format!("consult_{}", match self.target_agent {
            AgentType::Coder => "coder",
            AgentType::Researcher => "researcher",
            AgentType::Reasoner => "reasoner",
            AgentType::Planner => "planner",
            AgentType::Reviewer => "reviewer",
            AgentType::GeneralChat => "chat",
        })
    }

    fn description(&self) -> String {
        format!(
            "Consult the {:?} agent for specialized assistance. 
            Use this when you need a second opinion or help from a specialized peer. 
            Input should be a concise query or task description.",
            self.target_agent
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The specific question or task for the peer agent."
                },
                "context": {
                    "type": "string",
                    "description": "Optional background info to help the peer."
                }
            },
            "required": ["query"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "recursive",
            "notes": "Spawns a child agent turn. Governed by high-level supervisor budget.",
            "protocol": "A2A/SNS"
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let query = params["query"].as_str().ok_or_else(|| AgentError::Validation("Missing query".to_string()))?;
        let context = params["context"].as_str();

        let mut interaction = AgentInteraction::new(
            AgentType::GeneralChat, // Dummy source, will be improved in future refinement
            self.target_agent,
            query
        );

        if let Some(ctx) = context {
            interaction.trace_context.push(ctx.to_string());
        }

        info!("A2A: Consulting {:?}...", self.target_agent);
        let response = self.bridge.peer_call(interaction).await?;

        if response.success {
            Ok(ToolOutput::success(
                json!({ "answer": response.answer, "thought": response.thought }),
                format!("Response from {:?}:\n{}", self.target_agent, response.answer)
            ))
        } else {
            Ok(ToolOutput::failure(format!("Peer agent {:?} failed: {}", self.target_agent, response.answer)))
        }
    }
}

pub struct RemoteAgencyTool {
    client: reqwest::Client,
}

impl RemoteAgencyTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Tool for RemoteAgencyTool {
    fn name(&self) -> String {
        "dial_remote_agency".to_string()
    }

    fn description(&self) -> String {
        "Dial a remote Agency server over the internet. \n            Use this to collaborate with external agent swarms. \n            Requires the URL of the remote Nexus and the target agent role (e.g. 'coder', 'researcher').".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The base URL of the remote agency (e.g. https://api.nexus.io)" },
                "target_agent": { "type": "string", "enum": ["coder", "researcher", "reasoner", "chat"], "description": "The remote role to consult." },
                "query": { "type": "string", "description": "The task or query for the remote agency." }
            },
            "required": ["url", "target_agent", "query"]
        })
    }

    fn work_scope(&self) -> Value {
        json!({
            "status": "external",
            "network": "required",
            "protocol": "A2A/JSON-over-HTTP"
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let url = params["url"].as_str().ok_or_else(|| AgentError::Validation("Missing URL".to_string()))?;
        let target_str = params["target_agent"].as_str().unwrap_or("chat");
        let query = params["query"].as_str().ok_or_else(|| AgentError::Validation("Missing query".to_string()))?;

        let target_agent = match target_str {
            "coder" => AgentType::Coder,
            "researcher" => AgentType::Researcher,
            "reasoner" => AgentType::Reasoner,
            _ => AgentType::GeneralChat,
        };

        let interaction = AgentInteraction::new(AgentType::GeneralChat, target_agent, query);
        let endpoint = format!("{}/v1/a2a/interact", url.trim_end_matches('/'));

        info!("A2A: Dialing remote agency at {}...", url);
        
        let response = self.client.post(&endpoint)
            .json(&interaction)
            .send()
            .await
            .map_err(|e| AgentError::Tool(format!("Network error dialing remote: {}", e)))?;

        if response.status().is_success() {
            let res_body: AgentResponse = response.json().await
                .map_err(|e| AgentError::Tool(format!("Failed to parse remote response: {}", e)))?;
            
            Ok(ToolOutput::success(
                json!({ "answer": res_body.answer }),
                format!("Remote Response from {}:\n{}", url, res_body.answer)
            ))
        } else {
            Ok(ToolOutput::failure(format!("Remote agency at {} returned error: {}", url, response.status())))
        }
    }
}

                    
                    pub struct AnonymousAgencyTool {
                        dialer: Arc<Mutex<Option<crate::orchestrator::arti_a2a::AnonymousDialer>>>,
                    }
                    
                    impl AnonymousAgencyTool {
                        pub fn new() -> Self {
                            Self {
                                dialer: Arc::new(Mutex::new(None)),
                            }
                        }
                    }
                    
                    #[async_trait]
                    impl Tool for AnonymousAgencyTool {
                        fn name(&self) -> String {
                            "dial_anonymous_agency".to_string()
                        }
                    
                        fn description(&self) -> String {
                            "Dial an external Agency server anonymously via Tor (Arti). \n            Ensures your host identity is hidden. The remote agency will only see your Capability Identity. \n            Use this for high-privacy collaborations.".to_string()
                        }
                    
                        fn parameters(&self) -> Value {
                            json!({
                                "type": "object",
                                "properties": {
                                    "url": { "type": "string", "description": "The base URL or .onion address of the remote agency." },
                                    "target_agent": { "type": "string", "enum": ["coder", "researcher", "reasoner", "chat"], "description": "The remote role to consult." },
                                    "query": { "type": "string", "description": "The task or query for the remote agency." },
                                    "capability_role": { "type": "string", "description": "The role you want to present as (e.g. 'Senior Auditor')." }
                                },
                                "required": ["url", "target_agent", "query"]
                            })
                        }
                    
                        fn work_scope(&self) -> Value {
                            json!({
                                "status": "stealth",
                                "network": "tor/arti",
                                "anonymity": "high",
                                "protocol": "A2A/SNS/Onion"
                            })
                        }
                    
                        async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
                            let url = params["url"].as_str().ok_or_else(|| AgentError::Validation("Missing URL".to_string()))?;
                            let target_str = params["target_agent"].as_str().unwrap_or("chat");
                            let query = params["query"].as_str().ok_or_else(|| AgentError::Validation("Missing query".to_string()))?;
                            let cap_role = params["capability_role"].as_str().unwrap_or("Anonymous Agent");
                    
                            let target_agent = match target_str {
                                "coder" => AgentType::Coder,
                                "researcher" => AgentType::Researcher,
                                "reasoner" => AgentType::Reasoner,
                                _ => AgentType::GeneralChat,
                            };
                    
                            let interaction = AgentInteraction::new(AgentType::GeneralChat, target_agent, query);
                            
                            // Lazy-init the Tor client
                            let mut dialer_lock = self.dialer.lock().await;
                            if dialer_lock.is_none() {
                                *dialer_lock = Some(crate::orchestrator::arti_a2a::AnonymousDialer::new().await?);
                            }
                            let dialer = dialer_lock.as_ref().unwrap();
                    
                            let identity = crate::orchestrator::arti_a2a::CapabilityIdentity {
                                role: cap_role.to_string(),
                                credentials: vec!["standard-v1".to_string()],
                                reputation_score: 0.95,
                            };
                    
                            let response = dialer.anonymous_call(url, interaction, Some(identity)).await?;
                    
                            Ok(ToolOutput::success(
                                json!({ "answer": response.answer }),
                                format!("Anonymous Remote Response (via Tor):\n{}", response.answer)
                            ))
                        }
                    }
                    
