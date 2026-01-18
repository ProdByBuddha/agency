use serde::{Deserialize, Serialize};
use std::fs::File;
use std::collections::HashMap;

/// FPF-aligned Scale Classes (C.18.1 SLL)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum ScaleClass {
    /// Tiny: 1-bit or sub-1b models, low context, sub-second latency.
    Tiny,
    /// Standard: 3b models, balanced performance.
    Standard,
    /// Heavy: 7b+ models or high-cycle reasoning.
    Heavy,
    /// Logic: Rapid classification or safety logic.
    Logic,
}

impl ScaleClass {
    /// FPF Principle: Escalation Path (C.18.2)
    /// Returns the next stronger scale class, or the current one if already at maximum.
    pub fn escalate(&self) -> Self {
        match self {
            ScaleClass::Logic => ScaleClass::Tiny,
            ScaleClass::Tiny => ScaleClass::Standard,
            ScaleClass::Standard => ScaleClass::Heavy,
            ScaleClass::Heavy => ScaleClass::Heavy,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    models: Vec<serde_json::Value>,
    #[serde(default)]
    defaults: HashMap<String, String>,
}

/// FPF Scaling-Law Lens (SLL)
/// 
/// Captures the Scale Variables (S) that govern task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleProfile {
    pub class: ScaleClass,
    /// S: Predicted token/compute cost
    pub predicted_complexity: f32,
    /// χ: Scale Elasticity (rising, knee, flat)
    pub elasticity: String,
    pub target_model: String,
}

impl ScaleProfile {
    pub fn new(complexity: f32, vram_available_gb: f32) -> Self {
        // Load defaults from registry
        let defaults = if let Ok(file) = File::open("config/agency_models.json") {
            if let Ok(registry) = serde_json::from_reader::<_, Registry>(file) {
                registry.defaults
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        // FPF Integration: Scaling-Law Lens (SLL)
        // Complexity mapped to scale classes
        let (class, model_key, elasticity) = if complexity < 0.15 {
            // Very simple strings/greetings now default to Logic/Tiny
            (ScaleClass::Logic, "tiny", "flat")
        } else if complexity < 0.3 {
            (ScaleClass::Tiny, "tiny", "flat")
        } else if complexity < 0.7 {
            (ScaleClass::Standard, "standard", "rising")
        } else {
            (ScaleClass::Heavy, "heavy", "knee")
        };

        let target_model = defaults.get(model_key).cloned().unwrap_or_else(|| {
            match class {
                ScaleClass::Logic => "qwen2.5-coder:0.5b".to_string(),
                ScaleClass::Tiny => "qwen2.5-coder:0.5b".to_string(),
                ScaleClass::Standard => "qwen2.5:3b-q4".to_string(),
                ScaleClass::Heavy => if vram_available_gb >= 8.0 { "qwen2.5:7b-q4".to_string() } else { "qwen2.5:3b-q4".to_string() },
            }
        });

        Self {
            class,
            predicted_complexity: complexity,
            elasticity: elasticity.to_string(),
            target_model,
        }
    }

    pub fn new_with_class(class: ScaleClass, vram_available_gb: f32) -> Self {
        let defaults = if let Ok(file) = File::open("config/agency_models.json") {
            if let Ok(registry) = serde_json::from_reader::<_, Registry>(file) {
                registry.defaults
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let model_key = match class {
            ScaleClass::Logic => "tiny",
            ScaleClass::Tiny => "tiny",
            ScaleClass::Standard => "standard",
            ScaleClass::Heavy => "heavy",
        };

        let target_model = defaults.get(model_key).cloned().unwrap_or_else(|| {
            match class {
                ScaleClass::Logic => "qwen2.5-coder:0.5b".to_string(),
                ScaleClass::Tiny => "qwen2.5-coder:0.5b".to_string(),
                ScaleClass::Standard => "qwen2.5:3b-q4".to_string(),
                ScaleClass::Heavy => if vram_available_gb >= 8.0 { "qwen2.5:7b-q4".to_string() } else { "qwen2.5:3b-q4".to_string() },
            }
        });

        Self {
            class,
            predicted_complexity: 1.0, // Forced escalation
            elasticity: "flat".to_string(),
            target_model,
        }
    }

    pub fn format_for_audit(&self) -> String {
        format!(
            "SLL PROFILE: Class={:?}, Complexity={:.2}, Elasticity={}, Model={}",
            self.class, self.predicted_complexity, self.elasticity, self.target_model
        )
    }
}
