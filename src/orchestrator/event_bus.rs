//! Internal Event Bus for Agency Coordination
//! 
//! Provides a centralized, asynchronous pub/sub system for cross-component 
//! communication and telemetry tracing.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use std::sync::Arc;
use crate::agent::{LadeQuadrant, PubCharacteristic};

/// FPF-Aligned Claim Entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FPFBoundClaim {
    pub quadrant: LadeQuadrant,
    pub claim_id: String,
    pub content: String,
}

/// Global Agency Events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum AgencyEvent {
    /// FPF Boundary Crossing / Claim Event (A.6.B)
    BoundaryCrossing(FPFBoundClaim),
    /// FPF Publication Characteristic Update (E.17.5.5)
    PublicationUpdate { pc: PubCharacteristic },
    /// Discovered a new Markdown skill
    SkillDiscovered { name: String, version: String },
    /// A skill was promoted to the standard set
    SkillPromoted { name: String },
    /// Context compaction was triggered
    ContextCompacted { before_tokens: usize, after_tokens: usize },
    /// An agent turn started
    TurnStarted { agent: String, model: String },
    /// An agent turn ended
    TurnEnded { agent: String, success: bool, latency_ms: u128 },
    /// A tool call was initiated
    ToolCallStarted { tool: String },
    /// A tool call observation was received
    ToolCallFinished { tool: String, success: bool },
    /// HITL Approval was requested
    ApprovalRequested { id: String, tool: String },
    /// Generic system status update
    StatusUpdate(String),
}

pub struct EventBus {
    tx: broadcast::Sender<AgencyEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    /// Publish an event to all subscribers
    pub fn publish(&self, event: AgencyEvent) {
        let _ = self.tx.send(event);
    }

    /// Create a new subscriber
    pub fn subscribe(&self) -> broadcast::Receiver<AgencyEvent> {
        self.tx.subscribe()
    }
}

lazy_static::lazy_static! {
    /// Global singleton instance of the EventBus
    pub static ref AGENCY_EVENT_BUS: Arc<EventBus> = Arc::new(EventBus::new());
}

/// Helper macro to publish events globally
#[macro_export]
macro_rules! emit_event {
    ($event:expr) => {
        $crate::orchestrator::event_bus::AGENCY_EVENT_BUS.publish($event);
    };
}
