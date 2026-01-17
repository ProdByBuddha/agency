//! SOTA Semi-Autonomous Agency
//! 
//! A state-of-the-art Rust-based multi-agent system featuring:
//! - ReAct reasoning framework with tool calling
//! - Semantic vector memory (ChromaDB + fastembed)
//! - Multi-agent coordination with planning
//! - Self-reflection and error correction
//! - Safety guardrails
//! - Full session persistence

use rust_agency::tools::McpServer;
use anyhow::Result;
use std::sync::Arc;
use std::io::Write;
use tracing::info;
use tokio::sync::{Mutex, broadcast};

use rust_agency::memory::{Memory, VectorMemory, MemoryManager, EpisodicMemory};
use rust_agency::orchestrator::{Supervisor, SessionManager, profile::ProfileManager};
use rust_agency::agent::Speaker;
use rust_agency::tools::{
    Tool, ToolRegistry, WebSearchTool, CodeExecTool, MemoryQueryTool, 
    KnowledgeGraphTool, ArtifactTool, SandboxTool, CodebaseTool, 
    SystemTool, ForgeTool, VisualizationTool, 
    SpeakerRsTool, ScienceTool, ModelManager, VisionTool
};
use rust_agency::server::{run_server, AppState};

use rust_agency::orchestrator::uap_grpc::{UapGrpcWrapper, AgentServiceServer};
use tonic::transport::Server;

// ──────────────────────────────────────────────────────────────────────────────
// CONFIGURATION
// ──────────────────────────────────────────────────────────────────────────────

/// Configuration for the agency
struct AgencyConfig {
    /// Path to fallback memory file
    memory_file: String,
    /// Path to session persistence file
    session_file: String,
    /// Path to agency profile file
    profile_file: String,
}

impl Default for AgencyConfig {
    fn default() -> Self {
        Self {
            memory_file: "memory.json".to_string(),
            session_file: "session.json".to_string(),
            profile_file: "agency_profile.json".to_string(),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// MAIN ENTRY POINT
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // SOTA: Apply Process Hardening (codex-inspired)
    rust_agency::safety::hardening::apply_hardening();

    // Initialize tracing (logs)
    let _guard = rust_agency::utils::otel::init_telemetry("rust_agency")
        .expect("Failed to initialize OpenTelemetry");
    info!("🚀 Rust Agency Starting...");

    // ──────────────────────────────────────────────────────────────────────────
    // HARDENING: System Diagnostic Check
    // ──────────────────────────────────────────────────────────────────────────
    if let Err(e) = rust_agency::utils::hardening::SystemHardening::verify_environment().await {
        tracing::error!("🛑 Startup Failed: {}", e);
        std::process::exit(1);
    }

    // Load environment variables IMMEDIATELY
    dotenv::dotenv().ok();

    // ──────────────────────────────────────────────────────────────────────────
    // ORCHESTRATION: Integrated Microservices
    // ──────────────────────────────────────────────────────────────────────────
    if std::env::var("AGENCY_USE_REMOTE_MEMORY").unwrap_or_default() == "1" {
        tokio::spawn(async move {
            if let Err(e) = rust_agency::services::memory::run_memory_server().await {
                eprintln!("❌ Memory Server crashed: {}", e);
            }
        });
        
        // Wait for memory server to be ready
        let client = reqwest::Client::new();
        let port = std::env::var("AGENCY_MEMORY_PORT").unwrap_or_else(|_| "3001".to_string());
        let url = format!("http://localhost:{}/health", port);
        print!("⏳ Waiting for Memory Server...");
        for _ in 0..30 {
            if let Ok(res) = client.get(&url).send().await {
                if res.status().is_success() {
                    println!(" ✅ Ready!");
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            print!(".");
            std::io::stdout().flush().ok();
        }
    }

    if std::env::var("AGENCY_ENABLE_MOUTH").unwrap_or_default() == "1" {
        std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                if let Err(e) = rust_agency::services::speaker::run_speaker_server().await {
                    eprintln!("❌ Speaker Server crashed: {}", e);
                }
            });
        });

        let client = reqwest::Client::new();
        let port = std::env::var("AGENCY_SPEAKER_PORT").unwrap_or_else(|_| "3000".to_string());
        let url = format!("http://localhost:{}/health", port);
        print!("⏳ Waiting for Speaker Server...");
        for _ in 0..30 {
            if let Ok(res) = client.get(&url).send().await {
                if res.status().is_success() {
                    println!(" ✅ Ready!");
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            print!(".");
            std::io::stdout().flush().ok();
        }
    }

    if std::env::var("AGENCY_ENABLE_EARS").unwrap_or_default() == "1" {
        std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                if let Err(e) = rust_agency::services::listener::run_listener_server().await {
                    eprintln!("❌ Listener Server crashed: {}", e);
                }
            });
        });
    }

    // ──────────────────────────────────────────────────────────────────────────
    // UAP: Sovereign gRPC Grid
    // ──────────────────────────────────────────────────────────────────────────
    if std::env::var("AGENCY_ENABLE_UAP").unwrap_or_default() == "1" {
        // We'll use the dynamically configured provider for the UAP gRPC server
        let uap_provider = rust_agency::agent::dynamic_provider();
        let uap_tools = Arc::new(ToolRegistry::default());
        // For the UAP exposure, we create a default ReActAgent as the performer
        let uap_config = rust_agency::agent::AgentConfig::new(rust_agency::AgentType::GeneralChat, &rust_agency::orchestrator::profile::AgencyProfile::default());
        let uap_agent = Arc::new(rust_agency::agent::ReActAgent::new_with_provider(uap_provider, uap_config, uap_tools));
        
        let uap_service = UapGrpcWrapper::new(uap_agent);
        let uap_addr = std::env::var("AGENCY_UAP_ADDR").unwrap_or_else(|_| "0.0.0.0:50051".to_string()).parse()?;

        println!("🌐 Spawning UAP gRPC Server (Sovereign Grid) at {}...", uap_addr);
        tokio::spawn(async move {
            if let Err(e) = Server::builder()
                .add_service(AgentServiceServer::new(uap_service))
                .serve(uap_addr)
                .await 
            {
                eprintln!("❌ UAP gRPC Server crashed: {}", e);
            }
        });
    }

    // Check for CLI arguments
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--visualize" || args[1] == "-v") {
        let tool = VisualizationTool::new();
        let params = serde_json::json!({
            "output_file": args.get(2).map(|s| s.as_str()).unwrap_or("agency_isometric.json")
        });
        match tool.execute(params).await {
            Ok(res) => {
                println!("{}", res.summary);
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("Error generating visualization: {}", e);
                std::process::exit(1);
            }
        }
    }

    println!("\n{}", "═".repeat(60));
    println!("🚀 SOTA Semi-Autonomous Agency v0.2.0");
    println!("{}", "═".repeat(60));
    println!("Features: ReAct | Vector Memory | Multi-Agent | Planning | Telemetry");
    println!("{}\n", "═".repeat(60));

    let config = AgencyConfig::default();
    let start_local = chrono::Local::now().format("%H:%M:%S").to_string();
    
    // Initialize memory system
    let memory: Arc<dyn Memory> = Arc::new(
        VectorMemory::new(&config.memory_file)
            .expect("Failed to initialize memory system")
    );
    
    // Initialize MemoryManager for resource tracking
    let manager = Arc::new(MemoryManager::new(memory.clone()));
    
    // Initialize Episodic Memory for Chat History
    let episodic_memory = Arc::new(Mutex::new(EpisodicMemory::default()));

    // LLM Provider: Dynamic selection based on AGENCY_PROVIDER env var
    let provider = rust_agency::agent::dynamic_provider();

    // Initialize session persistence
    let session_manager = SessionManager::new(&config.session_file);

    // Initialize shared Speaker engine (Deduplicated SOTA)
    let shared_speaker = Arc::new(tokio::sync::Mutex::new(Speaker::new()?));

    // Initialize tools
    let tools = Arc::new(ToolRegistry::default());
    
    // SOTA: Concurrent Tool Registration (FPF Principle: Rapid Capability Establishment)
    tokio::join!(
        tools.register_instance(WebSearchTool::new()),
        tools.register_instance(CodeExecTool::new()),
        tools.register_instance(MemoryQueryTool::new(memory.clone())),
        tools.register_instance(KnowledgeGraphTool::new(memory.clone())),
        tools.register_instance(ArtifactTool::default()),
        tools.register_instance(SandboxTool::default()),
        tools.register_instance(CodebaseTool::default()),
        tools.register_instance(ModelManager),
        tools.register_instance(SpeakerRsTool::new(shared_speaker.clone())),
        tools.register_instance(VisualizationTool::new()),
        tools.register_instance(ScienceTool::new()),
        tools.register_instance(VisionTool::new()),
        tools.register_instance(ForgeTool::new("custom_tools", tools.clone())),
        tools.register_instance(SystemTool::new(manager.clone())),
        tools.register_instance(rust_agency::tools::WasmCompilerTool::new()),
        tools.register_instance(rust_agency::tools::WasmExecutorTool::new())
    );

    // SOTA: Markdown-Based Skill Discovery (pi-mono-inspired)
    if let Ok(skills) = rust_agency::tools::SkillLoader::discover_skills("skills").await {
        for skill in skills {
            let name = skill.name();
            tools.register_instance(skill).await;
            println!("📚 Discovered Skill: {}", name);
        }
    }

    // SOTA: Dynamic MCP Server Integration
    if let Ok(content) = std::fs::read_to_string("mcp_servers.json") {
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(servers) = config["servers"].as_array() {
                for server_cfg in servers {
                    let name = server_cfg["name"].as_str().unwrap_or("unnamed");
                    let command = server_cfg["command"].as_str().unwrap_or("");
                    let args: Vec<String> = server_cfg["args"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();

                    if !command.is_empty() {
                        match McpServer::spawn(name, command, &args).await {
                            Ok(server) => {
                                match tools.register_mcp_server(server).await {
                                    Ok(count) => println!("🔌 Connected to MCP Server '{}' ({} tools loaded)", name, count),
                                    Err(e) => tracing::warn!("Failed to register tools from MCP server '{}': {}", name, e),
                                }
                            }
                            Err(e) => tracing::warn!("Failed to spawn MCP server '{}': {}", name, e),
                        }
                    }
                }
            }
        }
    }

    // SOTA: Load all previously forged dynamic tools (Laboratory graduation)
    let _ = tools.load_dynamic_tools("standard_tools").await;
    if let Ok(count) = tools.load_dynamic_tools("custom_tools").await {
        if count > 0 {
            println!("🛠️  Loaded {} dynamic tools from laboratory ('custom_tools').", count);
        }
    }
    let profile_manager = ProfileManager::new(&config.profile_file);
    let profile = profile_manager.load().await.unwrap_or_default();
    println!("👤 Agency Profile loaded: {}", profile.name);

    // Initialize supervisor
    let mut supervisor = Supervisor::new_with_provider(provider.clone(), tools.clone())
        .await
        .with_memory(memory.clone())
        .with_session(session_manager)
        .with_episodic_memory(episodic_memory.clone())
        .with_profile(profile)
        .with_max_retries(2);

    // NOTE: Background thinking (CTM) is disabled by default to save resources on 16GB M2 Air.
    // To enable it, uncomment the following line or use the 'autonomous' command.
    // let _ = supervisor.activate_background_thinking().await;

    // Restore previous session
    if let Err(e) = supervisor.load_session().await {
        info!("Starting new session (previous session load failed or missing): {}", e);
    } else {
        println!("💾 Session restored from '{}'", config.session_file);
    }
    
    // Wrap Supervisor in Shared Mutex for Hybrid Access
    let shared_supervisor = Arc::new(Mutex::new(supervisor));

    // ──────────────────────────────────────────────────────────────────────────
    // AUTONOMY: Durable Task Processing
    // ──────────────────────────────────────────────────────────────────────────
    {
        let supervisor = shared_supervisor.clone();
        tokio::spawn(async move {
            info!("⚙️  Autonomy Engine Online (Background)");
            loop {
                // We lock briefly to check/process one task
                let processed = {
                    let mut guard = supervisor.lock().await;
                    guard.process_next_task().await.unwrap_or(false)
                };
                
                if !processed {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        });
    }

    // ──────────────────────────────────────────────────────────────────────────
    // SCHEDULER: Circadian Rhythm
    // ──────────────────────────────────────────────────────────────────────────
    {
        let supervisor_guard = shared_supervisor.lock().await;
        let queue = supervisor_guard.task_queue.clone();
        drop(supervisor_guard);

        let scheduler = rust_agency::orchestrator::scheduler::AgencyScheduler::new(queue)
            .await
            .expect("Failed to init scheduler");
        
        scheduler.init_defaults().await.expect("Failed to init habits");
        scheduler.start().await.expect("Failed to start scheduler");
        println!("⏰ Circadian Rhythm active.");
    }

    // ──────────────────────────────────────────────────────────────────────────
    // HOMEOSTASIS: Self-Regulation
    // ──────────────────────────────────────────────────────────────────────────
    {
        let supervisor_guard = shared_supervisor.lock().await;
        let semaphore = supervisor_guard.concurrency_limit.clone();
        drop(supervisor_guard);

        let homeostasis = rust_agency::orchestrator::homeostasis::HomeostasisEngine::new(semaphore, 4);
        tokio::spawn(async move {
            homeostasis.start().await;
        });
        println!("🌡️  Homeostasis active.");
    }

    // ──────────────────────────────────────────────────────────────────────────
    // HEALING: The Doctor
    // ──────────────────────────────────────────────────────────────────────────
    {
        let supervisor_guard = shared_supervisor.lock().await;
        let queue = supervisor_guard.task_queue.clone();
        drop(supervisor_guard);

        let doctor = rust_agency::orchestrator::healing::HealingEngine::new(queue);
        tokio::spawn(async move {
            doctor.start().await;
        });
        println!("👨‍⚕️  Self-Healing active.");
    }

    // ──────────────────────────────────────────────────────────────────────────
    // REMOTE EARS: Bidirectional Messaging
    // ──────────────────────────────────────────────────────────────────────────
    {
        let supervisor_guard = shared_supervisor.lock().await;
        let vocal_cords = supervisor_guard.vocal_cords.clone();
        let queue = supervisor_guard.task_queue.clone();
        drop(supervisor_guard);

        let vocal_cords_clone = vocal_cords.clone();
        tokio::spawn(async move {
            vocal_cords_clone.start_listening(queue).await;
        });
    }

    let (tx, _) = broadcast::channel(1024);

    // ──────────────────────────────────────────────────────────────────────────
    // HYBRID MODE: Spawn Server in Background
    // ──────────────────────────────────────────────────────────────────────────
    let server_state = AppState {
        provider: provider.clone(),
        start_local,
        speaker: shared_speaker.clone(),
        tx: tx.clone(),
        episodic_memory: episodic_memory.clone(),
        supervisor: shared_supervisor.clone(),
        current_task: Arc::new(Mutex::new(None)),
    };
    
    println!("🌐 Spawning Nexus Server (Background) at http://localhost:8002...");
    tokio::spawn(async move {
        if let Err(e) = run_server(server_state).await {
            eprintln!("❌ Server crashed: {}", e);
        }
    });

    // ──────────────────────────────────────────────────────────────────────────
    // LAUNCH INTERFACE
    // ──────────────────────────────────────────────────────────────────────────
    if std::env::var("AGENCY_DESKTOP").unwrap_or_default() == "1" {
        println!("🎨 Launching Desktop Interface (Tauri)...");
        rust_agency::desktop::run_desktop();
    } else {
        // Launch the professional FPF-aligned CLI with SHARED speaker and supervisor
        let cli = rust_agency::orchestrator::cli::AgencyCLI::new(shared_supervisor, shared_speaker);
        cli.run().await?;
    }

    Ok(())
}
