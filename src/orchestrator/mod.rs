//! Orchestrator Module
//! 
//! Coordinates multiple agents to solve complex tasks using
//! planning, routing, and supervision.

pub mod supervisor;
pub mod planner;
pub mod router;
pub mod session;
pub mod profile;
pub mod sns;
pub mod drr;
pub mod objective;
pub mod alignment;
pub mod role_algebra;
pub mod mht;
pub mod governance;
pub mod scale;
pub mod budget;
pub mod mvpk;
pub mod bridge;
pub mod service;
pub mod commitment;
pub mod aggregation;
pub mod provenance;
pub mod cn_frame;
pub mod kind;
pub mod evolution;
pub mod debt;
pub mod cli;
pub mod context;
pub mod optimal_info;
#[macro_use]
pub mod event_bus;
pub mod a2a;
pub mod arti_a2a;
pub mod uap_grpc;
pub mod queue;
pub mod scheduler;
pub mod sensory;
pub mod homeostasis;
pub mod vocal_cords;

pub use crate::agent::speaker_rs::Speaker;
pub use supervisor::{Supervisor, SupervisorResult};
pub use planner::{Planner, Plan, PlanStep};
pub use optimal_info::OptimalInfoSelector;
pub use router::{Router, RoutingDecision};
pub use session::{SessionManager, SessionState};
pub use drr::DesignRationaleRecord;
pub use objective::{Objective, ResourceBudget};
pub use alignment::{MethodDescription, MethodStep, WorkRecord, AssuranceLevel};
pub use role_algebra::RoleAlgebra;
pub use mht::{MHTEngine, MHTEvent};
pub use governance::{NormSquare, AdmissibilityGate, GateStatus, DeonticRule, DeonticModality, AdjudicationResult, AdjudicationVerdict};
pub use scale::{ScaleClass, ScaleProfile};
pub use budget::{AutonomyLedger, BudgetStatus};
pub use mvpk::Publication;
pub use bridge::Bridge;
pub use service::{ServiceClause, ServiceStatus};
pub use commitment::{Commitment, Modality, CommitmentStatus};
pub use aggregation::{Gamma, ResultPortfolio};
pub use provenance::EvidenceGraph;
pub use cn_frame::CNFrame;
pub use kind::{Kind, KindAlgebra};
pub use evolution::{EvolutionEvent, EvolutionEngine};
pub use debt::{HeuristicDebt, DebtRegistry};
pub use event_bus::{AGENCY_EVENT_BUS, AgencyEvent};
pub mod pai;
