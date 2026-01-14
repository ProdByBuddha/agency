# PAI Core Architecture: The Intelligence Fabric

This document details the technical implementation of the `pai-core` Rust port, focusing on concurrency, security, and integration patterns.

## 1. Deterministic Orchestration: `AlgorithmEngine`

The `AlgorithmEngine` is the heart of the PAI process. Unlike traditional LLM loops, it enforces a deterministic lifecycle through discrete phases.

### Concurrency Model
To support high-concurrency agent swarms, the engine uses **granular state locking**:
- **Phase & Effort**: Protected by `RwLock<T>` for independent read/write access.
- **Iteration Count**: Managed by `AtomicU32` for wait-free increments.
- **Requirements**: A shared `RwLock<Vec<ISCRequirement>>` allows multiple agents to check status simultaneously while serializing updates.

This design eliminates the "Global Lock" bottleneck, allowing the PAI logic to scale linearly with the number of CPU cores.

## 2. The Hook Pipeline: `HookManager`

The `HookManager` provides a non-invasive way to extend PAI behavior. It implements an asynchronous middleware pattern similar to high-performance web frameworks.

### Efficiency Strategy
- **Payload Pass-through**: Most hooks are "read-only" (e.g., Loggers, Validators). The `HookManager` avoids cloning the `HookEvent` unless a hook explicitly returns a `HookAction::Modify`.
- **Async Trait**: Hooks use `async-trait` to allow non-blocking network or I/O calls during event processing.

## 3. Adversarial Security: Layered Defense

We treat the AI's tool-use as a potential attack vector.

### The Oracle Sandbox
`VerificationOracle` acts as a security gateway for environmental checks. By using a command whitelist and protocol enforcement (HTTPS-only), it creates a "soft sandbox" that prevents the model from escaping its intended context.

### Content Redaction
`PrivacyGuard` implements a content-aware filter. It doesn't just block files; it actively sanitizes the communication stream by redacting entropy-based patterns like API keys and credentials before they are logged or sent to external providers.

## 4. Async Persistence: `TieredMemoryManager`

All I/O in `pai-core` is asynchronous to ensure the main agent loop remains responsive.
- **Storage**: Uses `tokio::fs` for non-blocking file operations.
- **Sanitization**: Paths are sanitized at the entry point to prevent `../` traversal attacks.
- **Snapshotting**: `RecoveryJournal` implements an "Atomic Write" pattern, ensuring that manual edits or critical changes are backed up to a restricted history directory before modification.

## 5. Integration with Agency

The port is integrated into the main `agency` orchestrator via the `PAIOrchestrator` bridge:
1. **Request Classification**: `EffortClassifier` determines the "Intelligence Tier" required.
2. **Identity Composition**: `AgentFactory` assembles the prompt fragments based on PAI traits.
3. **Execution**: The `Supervisor` executes the task, with `ReActAgent` triggering `PreToolUse` hooks in the PAI core for every action.
4. **Verification**: A skeptical "PAI Verifier" turn is triggered to cross-check results against the original requirements.
