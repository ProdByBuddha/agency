//! Continuous Thought Machine (CTM) Implementation
//! 
//! Inspired by SakanaAI's Continuous Thought Machines, this module implements 
//! an agentic version of internal temporal unfolding and neural synchronization.
//! 
//! Instead of a single reasoning step, the CTM unfolds its internal state over 
//! multiple "temporal cycles" before producing a synchronized output.

use anyhow::Result;
use ollama_rs::Ollama;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use std::sync::Arc;

use crate::agent::{AgentConfig, AgentType, SimpleAgent, LLMProvider, OllamaProvider, LLMCache, CachedProvider};
use crate::orchestrator::profile::AgencyProfile;
use crate::orchestrator::aggregation::RewardModel;

/// A single step in the internal temporal unfolding of a thought
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalStep {
    pub cycle: usize,
    pub internal_thought: String,
    pub confidence: f32,
    pub reward: Option<f32>,
}

/// The Continuous Thought Machine
#[derive(Clone)]
pub struct ContinuousThoughtMachine {
    agent: SimpleAgent,
    thought_buffer: Vec<TemporalStep>,
    max_cycles: usize,
    sync_threshold: f32,
    reward_model: Option<Arc<dyn RewardModel>>,
}

impl ContinuousThoughtMachine {
    pub fn new(
        ollama: Ollama,
        profile: &AgencyProfile,
    ) -> Self {
        let config = AgentConfig::new(AgentType::Reasoner, profile);
        let agent = SimpleAgent::new(ollama, config);
        
        Self {
            agent,
            thought_buffer: Vec::new(),
            max_cycles: 10,
            sync_threshold: 0.85,
            reward_model: None,
        }
    }

    pub fn with_reward_model(mut self, model: Arc<dyn RewardModel>) -> Self {
        self.reward_model = Some(model);
        self
    }

    #[allow(dead_code)]
    pub fn with_cache(mut self, cache: Arc<LLMCache>) -> Self {
        let ollama = Ollama::default(); 
        let provider = Arc::new(OllamaProvider::new(ollama)) as Arc<dyn LLMProvider>;
        let cached_provider = Arc::new(CachedProvider::new(provider, cache));
        self.agent = self.agent.with_provider(cached_provider);
        self
    }

    #[allow(dead_code)]
    pub fn with_provider(mut self, provider: Arc<dyn LLMProvider>) -> Self {
        self.agent = self.agent.with_provider(provider);
        self
    }

    #[allow(dead_code)]
    pub fn with_max_cycles(mut self, cycles: usize) -> Self {
        self.max_cycles = cycles;
        self
    }

    #[allow(dead_code)]
    pub fn with_sync_threshold(mut self, threshold: f32) -> Self {
        self.sync_threshold = threshold;
        self
    }

    /// Unfold the internal thought process over multiple temporal cycles
    pub async fn unfold(&mut self, query: &str, context: Option<&str>) -> Result<String> {
        self.thought_buffer.clear();
        info!("CTM unfolding thought process for: '{}'", query);

        for cycle in 1..=self.max_cycles {
            let thought = self.execute_cycle(query, cycle, context).await?;
            
            let confidence = self.evaluate_synchronization(&thought).await?;
            
            // SOTA: RLM Reward Integration
            let mut reward = None;
            if let Some(ref rm) = self.reward_model {
                let candidate = crate::orchestrator::aggregation::Candidate {
                    agent_id: "CTM_Cycle".to_string(),
                    answer: thought.clone(),
                    quality_score: 0.5,
                    risk_score: 0.1,
                    novelty_score: 0.0,
                    cost_tokens: 0,
                    assurance: crate::orchestrator::AssuranceLevel::L0,
                    reward_score: None,
                };
                if let Ok(scores) = rm.score(query, &[candidate]).await {
                    reward = scores.first().cloned();
                }
            }

            let step = TemporalStep {
                cycle,
                internal_thought: thought.clone(),
                confidence,
                reward,
            };
            
            self.thought_buffer.push(step);
            
            debug!(
                "Cycle {} Confidence: {:.2}, Reward: {}", 
                cycle, 
                confidence, 
                reward.map(|r| format!("{:.2}", r)).unwrap_or_else(|| "N/A".to_string())
            );

            // Optimization: Synchronize if confidence AND reward are high
            let sync_ready = confidence >= self.sync_threshold;
            let reward_ready = reward.map(|r| r >= 0.8).unwrap_or(true);

            if sync_ready && reward_ready && cycle >= 3 {
                info!("CTM synchronized and validated at cycle {} with confidence {:.2}", cycle, confidence);
                break;
            }
        }

        self.produce_synchronized_output(query).await
    }

    async fn execute_cycle(&self, query: &str, cycle: usize, context: Option<&str>) -> Result<String> {
        let history = self.thought_buffer
            .iter()
            .map(|s| format!("Cycle {}: {}", s.cycle, s.internal_thought))
            .collect::<Vec<_>>()
            .join("\n");

        let context_str = context.map(|c| format!("\nCONTEXT:\n{}\n", c)).unwrap_or_default();

        let prompt = format!(
            r###"INTERNAL TEMPORAL AXIS - CYCLE {} 
TASK: {} 
{}
PREVIOUS INTERNAL STATES:
{} 

INSTRUCTIONS:
Refine the internal representation of the solution. 
If this is an early cycle, explore possibilities. 
If this is a later cycle, converge on the most optimal and accurate thought.
BE EXTREMELY CONCISE. This is an internal thought, not a final answer.
"###,
            cycle, query, context_str, history
        );

        let response = self.agent.execute_simple(&prompt, None).await?;
        Ok(response.answer)
    }

    async fn evaluate_synchronization(&self, current_thought: &str) -> Result<f32> {
        if self.thought_buffer.is_empty() {
            return Ok(0.5); // Initial confidence
        }

        let last_thought = &self.thought_buffer.last().unwrap().internal_thought;
        
        // Use a very fast LLM call to judge "synchronization" (convergence)
        let prompt = format!(
            r###"Compare these two sequential internal thoughts. 
Are they converging on a stable conclusion?
Provide a synchronization score between 0.0 and 1.0.
0.0 = Completely different/divergent
1.0 = Perfectly synchronized/stable

THOUGHT A: {} 
THOUGHT B: {} 

SCORE:"###,
            last_thought, current_thought
        );

        let response = self.agent.execute_simple(&prompt, None).await?;
        let score_text = response.answer.trim();
        
        // Simple parsing of the score
        let score = score_text.split_whitespace()
            .find_map(|s| s.parse::<f32>().ok())
            .unwrap_or(0.7);

        Ok(score)
    }

    async fn produce_synchronized_output(&self, query: &str) -> Result<String> {
        let final_thought = self.thought_buffer.last() 
            .map(|s| s.internal_thought.as_str())
            .unwrap_or("No internal state reached.");

        let prompt = format!(
            r###"Based on the following internal temporal unfolding, provide the final synchronized answer to the query.

QUERY: {} 

SYNCHRONIZED INTERNAL STATE:
{} 

FINAL ANSWER:"###,
            query, final_thought
        );

        let response = self.agent.execute_simple(&prompt, None).await?;
        Ok(response.answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockCTMProvider;

    #[async_trait]
    impl LLMProvider for MockCTMProvider {
        async fn generate(&self, _model: &str, _prompt: String, _system: Option<String>) -> anyhow::Result<String> {
            Ok("This is a mock response from the CTM provider.".to_string())
        }

        async fn generate_stream(&self, _model: &str, _prompt: String, _system: Option<String>) -> anyhow::Result<futures::stream::BoxStream<'static, anyhow::Result<String>>> {
            use futures::stream::StreamExt;
            let stream = futures::stream::iter(vec![Ok("This is a mock response from the CTM provider.".to_string())]);
            Ok(stream.boxed())
        }

        fn get_lock(&self) -> std::sync::Arc<tokio::sync::Mutex<()>> {
            std::sync::Arc::new(tokio::sync::Mutex::new(()))
        }
    }

    #[tokio::test]
    async fn test_ctm_unfold() {
        let profile = AgencyProfile::default();
        let mut ctm = ContinuousThoughtMachine::new(Ollama::default(), &profile)
            .with_provider(Arc::new(MockCTMProvider))
            .with_max_cycles(3)
            .with_sync_threshold(0.8);
            
        let result = ctm.unfold("What is the meaning of life?", None).await.unwrap();
        assert!(result.contains("mock response"));
        assert!(ctm.thought_buffer.len() >= 1);
    }
}
