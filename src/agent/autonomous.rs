use anyhow::Result;
use ollama_rs::Ollama;
use std::sync::Arc;
use tracing::{info, warn};

use crate::agent::{ReActAgent, ReActStep, AgentConfig, AgentType, AgentResponse, Agent, LLMCache, LLMProvider, OllamaProvider, CachedProvider, NQDPortfolio};
use crate::agent::rl::{Experience, ExperienceBuffer};
use crate::orchestrator::profile::AgencyProfile;
use crate::orchestrator::{Objective, MethodDescription, AutonomyLedger};
use crate::orchestrator::aggregation::{RewardModel, Candidate};
use crate::tools::ToolRegistry;

/// A machine that thinks continuously towards a goal
pub struct AutonomousMachine {
    agent: ReActAgent,
    objective: Objective,
    method: MethodDescription,
    portfolio: NQDPortfolio,
    autonomy_ledger: AutonomyLedger,
    steps: Vec<ReActStep>,
    current_cycle: usize,
    reward_model: Option<Arc<dyn RewardModel>>,
    pub experience_buffer: ExperienceBuffer,
}

impl AutonomousMachine {
    pub fn new(ollama: Ollama, tools: Arc<ToolRegistry>, profile: &AgencyProfile, objective: Objective) -> Self {
        let config = AgentConfig::new(AgentType::Coder, profile);
        let agent = ReActAgent::new(ollama, config, tools);
        
        // FPF Integration: Initialize the abstract Method
        let method = MethodDescription::new(&objective.goal, &objective.goal);

        Self {
            agent,
            objective,
            method,
            portfolio: NQDPortfolio::new(),
            autonomy_ledger: AutonomyLedger::new(),
            steps: Vec::new(),
            current_cycle: 0,
            reward_model: None,
            experience_buffer: ExperienceBuffer::new(100),
        }
    }

    pub fn new_with_provider(provider: Arc<dyn LLMProvider>, tools: Arc<ToolRegistry>, profile: &AgencyProfile, objective: Objective) -> Self {
        let config = AgentConfig::new(AgentType::Coder, profile);
        let agent = ReActAgent::new_with_provider(provider, config, tools);
        
        let method = MethodDescription::new(&objective.goal, &objective.goal);

        Self {
            agent,
            objective,
            method,
            portfolio: NQDPortfolio::new(),
            autonomy_ledger: AutonomyLedger::new(),
            steps: Vec::new(),
            current_cycle: 0,
            reward_model: None,
            experience_buffer: ExperienceBuffer::new(100),
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

    pub fn with_provider(mut self, provider: Arc<dyn LLMProvider>) -> Self {
        self.agent = self.agent.with_provider(provider);
        self
    }

    pub fn get_method_id(&self) -> String {
        self.method.id.clone()
    }

    pub fn get_method(&self) -> MethodDescription {
        self.method.clone()
    }

    pub async fn run_iteration(&mut self) -> Result<AgentResponse> {
        self.current_cycle += 1;
        info!("Autonomous Machine thinking (Cycle {}): {}", self.current_cycle, self.objective.goal);
        
        // Check RoC Budget Status (E.16)
        let budget_status = self.autonomy_ledger.check_status(&self.objective.resource_budget);
        if budget_status.is_exhausted {
            warn!("RoC Budget Exhausted! Aborting mission iterations.");
            return Err(anyhow::anyhow!("AUTONOMY BUDGET EXHAUSTED: Max cycles, tokens, or time reached."));
        }

        // Use formal FPF objective, NQD Portfolio, and Autonomy Ledger in the prompt
        let objective_prompt = self.objective.format_for_prompt();
        let portfolio_prompt = self.portfolio.format_for_prompt();
        let ledger_prompt = self.autonomy_ledger.format_for_prompt(&self.objective.resource_budget);
        
        let jitter_hint = if self.current_cycle > 1 {
            format!("\n(MISSION RE-ATTEMPT {} - DIVERSIFY YOUR APPROACH.)\n", self.current_cycle)
        } else {
            "".to_string()
        };

        let final_query = format!("{}\n{}\n{}\n{}\nExecute the next steps to satisfy the acceptance criteria.", 
            objective_prompt, portfolio_prompt, ledger_prompt, jitter_hint);

        let mut response = self.agent.execute(&final_query, None).await?;
        
        // SOTA: Calculate Reinforcement Rewards (Phase 3)
        let mut nqd_scores = Vec::new();
        for step in &response.steps {
            let score = self.portfolio.evaluate_action(&step.actions, if response.success { 1.0 } else { 0.5 });
            nqd_scores.push(score);
        }

        // Combine RLM Extrinsic Reward with NQD Intrinsic Curiosity
        let mut extrinsic_reward = 0.0;
        let mut total_reward = 0.0;
        let mut intrinsic_reward = 0.0;

        if let Some(ref rm) = self.reward_model {
            let candidate = Candidate {
                agent_id: "Autonomous".to_string(),
                answer: response.answer.clone(),
                quality_score: if response.success { 0.8 } else { 0.1 },
                risk_score: 0.1,
                novelty_score: 0.0,
                cost_tokens: response.cost_tokens,
                assurance: crate::orchestrator::AssuranceLevel::L2,
                reward_score: None,
                scale_elasticity: crate::orchestrator::aggregation::ScaleElasticity::Unknown,
            };
            if let Ok(scores) = rm.score(&self.objective.goal, &[candidate]).await {
                if let Some(&er) = scores.first() {
                    extrinsic_reward = er;
                    // Total Reward = Extrinsic + Alpha * Intrinsic(Novelty + Diversity)
                    intrinsic_reward = if nqd_scores.is_empty() { 0.0 } else {
                        nqd_scores.iter().map(|s| (s.novelty + s.diversity) / 2.0).sum::<f32>() / nqd_scores.len() as f32
                    };
                    
                    total_reward = extrinsic_reward + (0.2 * intrinsic_reward);
                    info!("Iteration Cycle {} Total Reward: {:.4} (Extrinsic: {:.2}, Intrinsic: {:.2})", 
                        self.current_cycle, total_reward, extrinsic_reward, intrinsic_reward);
                    
                    response.reliability = total_reward;
                }
            }
        }

        // SOTA: Record experience for RL training (Phase 4)
        self.experience_buffer.record(Experience {
            query: self.objective.goal.clone(),
            steps: response.steps.clone(),
            answer: response.answer.clone(),
            total_reward,
            extrinsic_reward,
            intrinsic_reward,
        });

        // FPF Integration: Update Ledger and Portfolio
        self.autonomy_ledger.record_tokens(final_query.len() as u32 / 4 + response.answer.len() as u32 / 4);
        
        for step in &response.steps {
            for _ in &step.actions {
                self.autonomy_ledger.record_tool_call();
            }
            self.method = self.method.clone().with_step(&step.thought, vec!["Coder".to_string()]);
        }

        self.steps.extend(response.steps.clone());
        Ok(response)
    }
}