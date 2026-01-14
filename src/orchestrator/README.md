# Orchestrator

The coordination and governance layer of the Agency. This module is responsible for high-level "will" and multi-agent synergy.

## 🛰️ Supervisor (`supervisor.rs`)

The central nervous system. It orchestrates the lifecycle of a query:
1. **Routing**: Directs queries to the optimal agent portfolio.
2. **Escalation**: Automatically retries failed tasks with more powerful models or different strategies.
3. **Verification**: Closes the loop by adjudicating agent outputs against acceptance criteria.

## 📋 Planning & Routing (`planner.rs`, `router.rs`)

- **Task Decomposition**: Breaks complex goals into discrete `PlanSteps`.
- **Optimal Info Selection**: Implements Decision Sensitivity logic to resolve plan-critical uncertainties before execution.
- **Scaling-Law Lens (SLL)**: Predicts task complexity and selects the smallest sufficient model to minimize resource consumption.

## 🤝 A2A Collaboration (`a2a.rs`, `arti_a2a.rs`)

Implements the Agent-to-Agent communication protocol. Agents can consult specialized peers locally via `PeerAgentTool` or remotely via `RemoteAgencyTool`.

- **Anonymous Capability Dialing**: Uses **Tor (Arti)** to establish encrypted, identity-less tunnels for A2A. The `AnonymousAgencyTool` allows agents to interact with remote swarms without broadcasting their host identity, presenting only a **CapabilityIdentity** (role, reputation, and credentials).
- **SNS Protocol**: High-efficiency recursive context passing.

## ⚖️ Governance & Audit

- **DRR (Design-Rationale Record) (`drr.rs`)**: Automatically records the "Why" behind every major system decision.
- **Autonomy Ledger (`budget.rs`)**: Tracks token and time usage to ensure "Responsibly Local" execution.
- **Event Bus (`event_bus.rs`)**: Centralized telemetry for all cross-component communication.

## 🏛️ CLI & UI (`cli.rs`, `server.rs`)

Provides professional interfaces for both command-line interaction and high-fidelity dashboard visualization.
