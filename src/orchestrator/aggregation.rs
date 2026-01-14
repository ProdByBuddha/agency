use serde::{Deserialize, Serialize};
use crate::orchestrator::alignment::AssuranceLevel;
use async_trait::async_trait;
use crate::agent::LLMProvider;
use std::sync::Arc;

/// FPF-aligned Candidate for Portfolio Selection (G.5)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub agent_id: String,
    pub answer: String,
    /// Q - Quality/Use-Value (C.17)
    pub quality_score: f32,
    /// R - Risk/Constraint-Fit (C.17)
    pub risk_score: f32,
    /// N - Novelty (C.17)
    pub novelty_score: f32,
    /// C - Cost tokens (Resrc-CAL)
    pub cost_tokens: u32,
    pub assurance: AssuranceLevel,
    /// Reward score provided by the Reinforcement Model (RLM)
    pub reward_score: Option<f32>,
}

/// FPF-aligned Result Portfolio (G.9)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResultPortfolio {
    pub candidates: Vec<Candidate>,
    pub selected_index: Option<usize>,
}

/// Interface for Reinforcement Learning Reward Models
#[async_trait]
pub trait RewardModel: Send + Sync {
    /// Score a set of candidates against the original query
    async fn score(&self, query: &str, candidates: &[Candidate]) -> anyhow::Result<Vec<f32>>;
}

/// A Reward Model that uses an LLM to judge and score candidates
pub struct LLMRewardModel {
    provider: Arc<dyn LLMProvider>,
    model: String,
}

impl LLMRewardModel {
    pub fn new(provider: Arc<dyn LLMProvider>, model: String) -> Self {
        Self { provider, model }
    }
}

#[async_trait]
impl RewardModel for LLMRewardModel {
    async fn score(&self, query: &str, candidates: &[Candidate]) -> anyhow::Result<Vec<f32>> {
        let mut scores = Vec::with_capacity(candidates.len());
        
        for candidate in candidates {
            let prompt = format!(
                r###"You are a Reward Model (RM) judging AI responses.
QUERY: {}
RESPONSE: {}

TASK:
Score the response quality on a scale from 0.0 to 1.0.
1.0 = Perfect, factually accurate, and follows all instructions.
0.0 = Completely wrong, hallucinated, or irrelevant.

Provide ONLY the numeric score."###,
                query, candidate.answer
            );

            let response = self.provider.generate(&self.model, prompt, None).await?;
            let score = response.trim().parse::<f32>().unwrap_or(0.5);
            scores.push(score);
        }

        Ok(scores)
    }
}

pub struct Gamma;

impl Gamma {
    /// FPF Standard: Pareto-Dominance Selection (A.0 - ParetoOnly default)
    /// Given a portfolio, select the candidate that optimizes the N-U-C-D Objective Vector.
    pub fn select_pareto_winner(portfolio: &ResultPortfolio) -> Option<usize> {
        if portfolio.candidates.is_empty() { return None; }

        let mut winner_idx = 0;
        let mut max_score = -1.0;

        for (i, c) in portfolio.candidates.iter().enumerate() {
            // Quality (U) x (1 - Risk (C)) x Novelty (N)
            let raw_quality = c.reward_score.unwrap_or(c.quality_score);
            let constraint_fit = 1.0 - c.risk_score;
            
            // Score = U * C_fit * (1 + N) / Cost_norm
            let cost_norm = 1.0 + (c.cost_tokens as f32 / 1000.0);
            let score = (raw_quality * constraint_fit * (1.0 + c.novelty_score)) / cost_norm;
            
            if score > max_score {
                max_score = score;
                winner_idx = i;
            }
        }

        Some(winner_idx)
    }

    /// Weakest-Link Roll-up for Assurance (B.3)
    pub fn roll_up_assurance(levels: &[AssuranceLevel]) -> AssuranceLevel {
        if levels.is_empty() { return AssuranceLevel::L0; }
        levels.iter().min_by_key(|&&l| l as u8).cloned().unwrap_or(AssuranceLevel::L0)
    }

    pub fn roll_up_costs(costs: &[u32]) -> u32 {
        costs.iter().sum()
    }

    pub fn all_succeeded(results: &[bool]) -> bool {
        !results.is_empty() && results.iter().all(|&r| r)
    }
}
