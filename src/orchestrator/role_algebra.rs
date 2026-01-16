use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// FPF-aligned U.RoleAlgebra (A.2.7)
/// 
/// Manages relations between roles: 
/// - Specialization (≤)
/// - Incompatibility (⊥)
/// - Bundles (⊗)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleAlgebra {
    /// Incompatibility set (Roles that cannot overlap)
    pub incompatible: HashSet<(String, String)>,
    /// Specialization map (Child -> Parent)
    pub specialization: Vec<(String, String)>,
}

impl RoleAlgebra {
    pub fn new() -> Self {
        let mut algebra = Self::default();
        
        // FPF Standard: Separation of Duties (SoD)
        // Performer is incompatible with Reviewer
        algebra.add_incompatibility("Performer", "Reviewer");
        algebra.add_incompatibility("Coder", "Reviewer");
        
        // FPF Standard: Specialization
        // All specialized agents are 'Agents'
        algebra.add_specialization("Coder", "Agent");
        algebra.add_specialization("Reasoner", "Agent");
        algebra.add_specialization("Researcher", "Agent");
        
        algebra
    }

    pub fn add_incompatibility(&mut self, a: impl Into<String>, b: impl Into<String>) {
        let (a_s, b_s) = (a.into(), b.into());
        self.incompatible.insert((a_s.clone(), b_s.clone()));
        self.incompatible.insert((b_s, a_s));
    }

    pub fn add_specialization(&mut self, child: impl Into<String>, parent: impl Into<String>) {
        self.specialization.push((child.into(), parent.into()));
    }

    /// Check if two roles are incompatible (⊥)
    pub fn is_incompatible(&self, role_a: &str, role_b: &str) -> bool {
        self.incompatible.contains(&(role_a.to_string(), role_b.to_string()))
    }

    /// Check if role_a satisfies role_b (≤)
    pub fn satisfies(&self, role_a: &str, role_b: &str) -> bool {
        if role_a == role_b { return true; }
        self.specialization.iter().any(|(c, p)| c == role_a && p == role_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_algebra_relations() {
        let algebra = RoleAlgebra::new();
        
        // Test Specialization
        assert!(algebra.satisfies("Coder", "Agent"));
        assert!(algebra.satisfies("Agent", "Agent"));
        assert!(!algebra.satisfies("Agent", "Coder"));

        // Test Incompatibility (SoD)
        assert!(algebra.is_incompatible("Coder", "Reviewer"));
        assert!(algebra.is_incompatible("Reviewer", "Coder"));
        assert!(!algebra.is_incompatible("Coder", "Agent"));
    }
}
