//! FPF A.6.B - Boundary Norm Square (L/A/D/E Routing)
//! 
//! Provides structural routing for boundary statements to prevent "contract soup"
//! and enable multi-view safety.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The four quadrants of the Boundary Norm Square
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryQuadrant {
    /// L - Laws & Definitions (What things mean)
    Law,
    /// A - Admissibility & Gates (What is allowed to cross)
    Admissibility,
    /// D - Deontics & Commitments (Who owes what)
    Deontic,
    /// E - Work-Effects & Evidence (What was actually observed)
    Evidence,
}

impl fmt::Display for BoundaryQuadrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Law => "L.Law",
            Self::Admissibility => "A.Gate",
            Self::Deontic => "D.Duty",
            Self::Evidence => "E.Fact",
        };
        write!(f, "{}", label)
    }
}

/// A structured claim aligned with FPF A.6
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryClaim {
    pub id: String,
    pub quadrant: BoundaryQuadrant,
    pub content: String,
    pub context_id: String,
    pub source_id: String,
}

impl BoundaryClaim {
    pub fn new(quadrant: BoundaryQuadrant, content: impl Into<String>, context: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            quadrant,
            content: content.into(),
            context_id: context.to_string(),
            source_id: "rust_agency".to_string(),
        }
    }
}

/// Helper to classify strings into quadrants (Lightweight RPR-SERV)
pub fn classify_statement(text: &str) -> BoundaryQuadrant {
    let t = text.to_lowercase();
    if t.contains("shall") || t.contains("must") || t.contains("owe") || t.contains("commit") {
        BoundaryQuadrant::Deontic
    } else if t.contains("allow") || t.contains("block") || t.contains("gate") || t.contains("permit") {
        BoundaryQuadrant::Admissibility
    } else if t.contains("observed") || t.contains("result") || t.contains("fact") || t.contains("evidence") {
        BoundaryQuadrant::Evidence
    } else {
        BoundaryQuadrant::Law // Default: Definition/Informative
    }
}
