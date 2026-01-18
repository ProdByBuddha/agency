use serde::{Deserialize, Serialize};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// SAP Sovereign Rule: A constitutional constraint for the Agency.
/// Aligns with Deontic Logic (Must, May, Prohibited).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SovereignRule {
    pub rule_id: String,
    pub description: String,
    pub modality: DeonticModality,
    pub priority: u8, // 0-255, where 255 is absolute
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DeonticModality {
    /// OBLIGATION: The agent MUST perform this action.
    Must,
    /// PERMISSION: The agent MAY perform this action.
    May,
    /// PROHIBITION: The agent MUST NOT perform this action.
    Prohibited,
}

/// SAP Alignment Audit: The result of checking an L3/L4 action against L5 rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignmentAudit {
    pub audit_id: String,
    pub timestamp: DateTime<Utc>,
    pub target_id: String, // ID of Task or Commitment
    pub score: f32, // 0.0 to 1.0 alignment score
    pub violations: Vec<String>,
    pub status: AuditStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuditStatus {
    Aligned,
    Flagged,
    Blocked,
    Overridden, // L5: The "Sovereign Lever" has been pulled
}

pub struct AlignmentEngine {
    rules: Vec<SovereignRule>,
}

impl AlignmentEngine {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// PAI Standard: Initialize with Constitutional defaults
    pub fn sovereign_defaults() -> Self {
        let mut engine = Self::new();
        engine.rules.push(SovereignRule {
            rule_id: "RULE_LOCAL_FIRST".to_string(),
            description: "Data MUST remain local unless explicitly authorized.".to_string(),
            modality: DeonticModality::Must,
            priority: 255,
        });
        engine.rules.push(SovereignRule {
            rule_id: "RULE_NO_CENTRALIZED_AUTH".to_string(),
            description: "Dependency on black-box centralized identity is PROHIBITED.".to_string(),
            modality: DeonticModality::Prohibited,
            priority: 200,
        });
        engine.rules.push(SovereignRule {
            rule_id: "RULE_REGENERATIVE_SURPLUS".to_string(),
            description: "Tasks SHOULD generate a capital surplus (DSGM).".to_string(),
            modality: DeonticModality::May,
            priority: 150,
        });
        engine
    }

    pub fn audit(&self, description: &str, metadata: &HashMap<String, String>, sovereign_lever: bool) -> AlignmentAudit {
        let mut violations = Vec::new();
        let mut score = 1.0;

        for rule in &self.rules {
            match rule.modality {
                DeonticModality::Prohibited => {
                    if description.to_lowercase().contains(&rule.rule_id.to_lowercase()) || 
                       metadata.values().any(|v| v.contains(&rule.rule_id)) {
                        violations.push(format!("Prohibition Violated: {}", rule.description));
                        score -= 0.5;
                    }
                }
                DeonticModality::Must => {
                    // Logic for ensuring mandatory conditions are met
                }
                _ => {}
            }
        }

        let status = if sovereign_lever {
            AuditStatus::Overridden
        } else if score < 0.5 {
            AuditStatus::Blocked
        } else if score < 1.0 {
            AuditStatus::Flagged
        } else {
            AuditStatus::Aligned
        };

        AlignmentAudit {
            audit_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            target_id: "pending".to_string(),
            score: if sovereign_lever { 1.0 } else { score },
            violations,
            status,
        }
    }
}
