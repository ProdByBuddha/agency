use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use anyhow::Result;
use chrono::{DateTime, Utc};
use crate::vcp::ValueCommitment;
use crate::sap::AlignmentAudit;

/// UAP Task: The high-level objective assigned to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UapTask {
    pub task_id: String,
    pub input: String,
    pub artifacts: Vec<UapArtifact>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: UapTaskStatus,
    /// L4 Extension: The economic commitment associated with this task
    pub commitment: Option<ValueCommitment>,
    /// L5 Extension: The result of the alignment audit
    pub audit: Option<AlignmentAudit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UapTaskStatus {
    Created,
    Running,
    Completed,
    Failed,
    BlockedByPolicy,
}

/// UAP Step: A single incremental action taken by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UapStep {
    pub step_id: String,
    pub task_id: String,
    pub name: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub status: UapStepStatus,
    pub artifacts: Vec<UapArtifact>,
    pub is_last: bool,
    pub phase_metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UapStepStatus {
    Created,
    Running,
    Completed,
    Failed,
}

/// UAP Artifact: A file or data object produced during a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UapArtifact {
    pub artifact_id: String,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub hash: Option<String>,
}

#[async_trait]
pub trait SovereignAgent: Send + Sync {
    async fn create_task(&self, input: &str, commitment: Option<ValueCommitment>) -> Result<UapTask>;
    async fn propose_commitment(&self, input: &str) -> Result<ValueCommitment>;
    async fn audit_alignment(&self, input: &str, override_lever: bool) -> Result<AlignmentAudit>;
    async fn execute_step(&self, task_id: &str, input: Option<serde_json::Value>) -> Result<UapStep>;
    async fn list_steps(&self, task_id: &str) -> Result<Vec<UapStep>>;
    async fn get_task(&self, task_id: &str) -> Result<UapTask>;
    async fn list_artifacts(&self, task_id: &str) -> Result<Vec<UapArtifact>>;
}

impl UapTask {
    pub fn new(input: &str) -> Self {
        let now = Utc::now();
        Self {
            task_id: Uuid::new_v4().to_string(),
            input: input.to_string(),
            artifacts: Vec::new(),
            created_at: now,
            updated_at: now,
            status: UapTaskStatus::Created,
            commitment: None,
            audit: None,
        }
    }
}

impl UapStep {
    pub fn new(task_id: &str, name: &str) -> Self {
        Self {
            step_id: Uuid::new_v4().to_string(),
            task_id: task_id.to_string(),
            name: name.to_string(),
            input: None,
            output: None,
            status: UapStepStatus::Created,
            artifacts: Vec::new(),
            is_last: false,
            phase_metadata: HashMap::new(),
        }
    }
}