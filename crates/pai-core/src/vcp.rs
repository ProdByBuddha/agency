use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// VCP Value Commitment: A formal economic contract issued by an agent.
/// This represents an L4 commitment to deliver a specific outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueCommitment {
    pub commitment_id: String,
    pub task_id: String, // Linked to UAP Task
    pub issuer_id: String, // The agent issuing the commitment
    pub created_at: DateTime<Utc>,
    pub status: CommitmentStatus,
    pub modality: CommitmentModality,
    pub impact: EconomicImpact,
    /// Sovereignty: The OTS hash anchoring this commitment to Bitcoin
    pub anchor_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommitmentStatus {
    Proposed,
    Active,
    Fulfilled,
    Breached,
    Liquidated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommitmentModality {
    /// Strict obligation: Must deliver or it's a system failure
    Imperative, 
    /// Best effort: Optimal value generation
    Aspirational,
    /// Contingent: Only if specific resources are allocated
    Conditional,
}

/// DSGM Alignment: Quantifying the transition to post-labor capital.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicImpact {
    /// Estimated human labor hours saved by this automated outcome
    pub labor_decoupling_score: f32,
    /// The amount of "AI Capital" (durable assets) created
    pub capital_generation_score: f32,
    /// Value assigned according to the user's specific Telos
    pub utility_value: f32,
    /// Metadata mapping to DSGM specific metrics
    pub dsgm_metrics: HashMap<String, String>,
}

/// Proof of Value: The L4 evidence required to fulfill a commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOfValue {
    pub proof_id: String,
    pub commitment_id: String,
    pub evidence_hashes: Vec<String>, // List of artifact hashes (UAP Artifacts)
    pub verification_method: String, // e.g., "Compiler Pass", "Pareto Swarm Consensus"
    pub verified_at: Option<DateTime<Utc>>,
}

impl ValueCommitment {
    pub fn new(task_id: &str, issuer_id: &str, modality: CommitmentModality) -> Self {
        Self {
            commitment_id: Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            issuer_id: issuer_id.to_string(),
            created_at: Utc::now(),
            status: CommitmentStatus::Proposed,
            modality,
            impact: EconomicImpact::default(),
            anchor_hash: None,
        }
    }

    pub fn fulfill(&mut self) {
        self.status = CommitmentStatus::Fulfilled;
    }
}

impl Default for EconomicImpact {
    fn default() -> Self {
        Self {
            labor_decoupling_score: 0.0,
            capital_generation_score: 0.0,
            utility_value: 0.0,
            dsgm_metrics: HashMap::new(),
        }
    }
}

/// Logic for calculating the "Regenerative Surplus" of a task
pub struct ValueCalculus;

impl ValueCalculus {
    pub fn calculate_surplus(labor_hours: f32, reuse_factor: f32) -> f32 {
        // DSGM formula: Value = Labor Saved * (1 + Future Reuse Potential)
        labor_hours * (1.0 + reuse_factor)
    }
}
