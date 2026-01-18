use serde::{Deserialize, Serialize};
use crate::orchestrator::{WorkRecord, governance::NormSquare, debt::DebtRegistry, ScaleClass, aggregation::ScaleElasticity};
use chrono::Utc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    /// PlainView: Final user-facing surface
    pub answer: String,
    /// TechView: Internal reasoning/projection surface
    pub thought: Option<String>,
    /// AssuranceLane: Reliability/R-Score surface
    pub reliability: f32,
    pub telemetry: Telemetry,
    pub rationale: Option<crate::orchestrator::DesignRationaleRecord>,
    pub governance: Option<NormSquare>,
    pub debt_register: Option<DebtRegistry>,
    /// HITL State: Present if awaiting human approval
    pub pending_approval: Option<crate::safety::ApprovalRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Telemetry {
    pub latency_ms: u128,
    pub tool_calls: usize,
    pub evidence_count: usize,
    /// FPF-aligned Scale Class (C.18.1)
    pub scale: ScaleClass,
    pub model: String,
    pub elasticity: ScaleElasticity,
}

impl Publication {
    pub fn project(
        answer: String, 
        work: &WorkRecord, 
        scale_profile: crate::orchestrator::ScaleProfile,
        square: Option<NormSquare>,
        rationale: Option<crate::orchestrator::DesignRationaleRecord>,
        debt_register: Option<DebtRegistry>
    ) -> Self {
        // Use exact field names from current ReActStep definition
        let tool_calls = work.trace.iter().map(|s| s.actions.len()).sum();
        let evidence_count = work.trace.iter().map(|s| s.observations.len()).sum();
        
        let end = work.end_time.unwrap_or_else(Utc::now);
        let latency = end.signed_duration_since(work.start_time).num_milliseconds().max(0) as u128;
        
        if latency == 0 {
            tracing::debug!("LATENCY_DEBUG: end={:?}, start={:?}, diff={:?}", end, work.start_time, end - work.start_time);
        }

        Self {
            answer,
            thought: None, // Will be populated by the caller
            reliability: 1.0, // Default SoTA reliability
            telemetry: Telemetry {
                latency_ms: latency,
                tool_calls,
                evidence_count,
                scale: scale_profile.class,
                model: scale_profile.target_model,
                elasticity: scale_profile.elasticity,
            },
            rationale,
            governance: square,
            debt_register,
            pending_approval: None,
        }
    }

    pub fn with_mvpk(mut self, thought: Option<String>, reliability: f32) -> Self {
        self.thought = thought;
        self.reliability = reliability;
        self
    }

    pub fn with_approval(mut self, approval: Option<crate::safety::ApprovalRequest>) -> Self {
        self.pending_approval = approval;
        self
    }

    pub fn format_full_audit(&self) -> String {
        let mut out = format!("✅ FINAL ANSWER (PlainView):\n{}\n\n", self.answer);
        
        if let Some(ref t) = self.thought {
            out.push_str(&format!("🧠 INTERNAL PROJECTION (TechView):\n{}\n\n", t));
        }

        out.push_str("🛠️  OPERATIONAL TELEMETRY (TechCard):\n");
        out.push_str(&format!("  - Latency: {}ms\n", self.telemetry.latency_ms));
        out.push_str(&format!("  - Tools: {}\n", self.telemetry.tool_calls));
        out.push_str(&format!("  - Evidence: {}\n", self.telemetry.evidence_count));
        out.push_str(&format!("  - Scale: {:?} (Elasticity: {:?})\n", self.telemetry.scale, self.telemetry.elasticity));
        out.push_str(&format!("  - Model: {}\n", self.telemetry.model));
        out.push_str(&format!("  - Reliability (R): {:.2}\n\n", self.reliability));
        
        if let Some(ref drr) = self.rationale {
            out.push_str("🛡️  GOVERNANCE SURFACE (AssuranceLane):\n");
            out.push_str(&format!("  - Context: {}\n", drr.context));
            out.push_str(&format!("  - Decision: {}\n\n", drr.decision));
        }

        out
    }
}
