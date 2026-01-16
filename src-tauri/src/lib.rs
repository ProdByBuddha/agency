use tauri::{Emitter, Manager};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use rust_agency::orchestrator::Supervisor;
use rust_agency::agent::{Speaker, LLMProvider};
use rust_agency::memory::{Memory, VectorMemory, MemoryManager, EpisodicMemory};
use rust_agency::orchestrator::{SessionManager, profile::ProfileManager};
use rust_agency::tools::{
    ToolRegistry, WebSearchTool, CodeExecTool, MemoryQueryTool, 
    KnowledgeGraphTool, ArtifactTool, SandboxTool, CodebaseTool, 
    SystemTool, ForgeTool, VisualizationTool, 
    SpeakerRsTool, ScienceTool, VisionTool, ModelManager
};

struct AgencyState {
    supervisor: Arc<Mutex<Supervisor>>,
    speaker: Arc<Mutex<Speaker>>,
    current_task: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    episodic_memory: Arc<Mutex<EpisodicMemory>>,
}

#[tauri::command]
async fn send_query(
    query: String, 
    state: tauri::State<'_, AgencyState>, 
    app: tauri::AppHandle
) -> Result<(), String> {
    let supervisor = state.supervisor.clone();
    let current_task = state.current_task.clone();
    let speaker = state.speaker.clone();
    let app_handle = app.clone();

    // Abort existing
    {
        let mut task_guard = current_task.lock().await;
        if let Some(handle) = task_guard.take() {
            handle.abort();
            app.emit("nexus-event", "STATE:ABORTED").unwrap();
        }
    }

    let handle = tokio::spawn(async move {
        app_handle.emit("nexus-event", "🚀 Request: Orchestrating Agency...").unwrap();
        
        let mut sup = supervisor.lock().await;
        let result = sup.handle(&query).await;

        match result {
            Ok(res) => {
                app_handle.emit("nexus-event", format!("FINAL_ANSWER:{}", res.answer)).unwrap();
                if let Some(pub_obj) = res.publication {
                    app_handle.emit("nexus-event", format!("RELIABILITY:{}", pub_obj.reliability)).unwrap();
                    let assurance = serde_json::json!({
                        "latency": pub_obj.telemetry.latency_ms,
                        "tools": pub_obj.telemetry.tool_calls,
                        "evidence": pub_obj.telemetry.evidence_count,
                        "scale": format!("{:?}", pub_obj.telemetry.scale),
                        "model": pub_obj.telemetry.model
                    });
                    app_handle.emit("nexus-event", format!("ASSURANCE:{}", assurance)).unwrap();
                }
                
                // Speak the answer
                let to_speak = res.answer.clone();
                let spk = speaker.clone();
                tokio::spawn(async move {
                    let mut s = spk.lock().await;
                    let _ = s.say(&to_speak).await;
                });
            }
            Err(e) => {
                app_handle.emit("nexus-event", format!("ANSWER:Error: {}", e)).unwrap();
            }
        }
        app_handle.emit("nexus-event", "STATE:TURN_COMPLETE").unwrap();
    });

    *state.current_task.lock().await = Some(handle.abort_handle());
    Ok(())
}

#[tauri::command]
async fn stop_inference(state: tauri::State<'_, AgencyState>, app: tauri::AppHandle) -> Result<(), String> {
    let mut task_guard = state.current_task.lock().await;
    if let Some(handle) = task_guard.take() {
        handle.abort();
        app.emit("nexus-event", "STATE:STOPPED").unwrap();
    }
    Ok(())
}

#[tauri::command]
async fn clear_memory(state: tauri::State<'_, AgencyState>) -> Result<(), String> {
    state.supervisor.lock().await.clear_history().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_log::Builder::default().build())
    .plugin(tauri_plugin_shell::init())
    .setup(|app| {
        // Initialize Core Infrastructure
        let handle = app.handle().clone();
        
        tauri::async_runtime::spawn(async move {
            // Load Env
            if let Ok(path) = std::env::current_dir() {
                println!("Working dir: {:?}", path);
            }
            // Manually load .env since we might be in a bundle
            dotenv::dotenv().ok();

            // Initialize Memory
            let memory_file = std::env::var("AGENCY_MEMORY_PATH").unwrap_or("memory.json".to_string());
            let memory: Arc<dyn Memory> = Arc::new(VectorMemory::new(&memory_file).unwrap());
            let manager = Arc::new(MemoryManager::new(memory.clone()));
            let episodic_memory = Arc::new(Mutex::new(EpisodicMemory::default()));

            // Provider
            let provider = rust_agency::agent::dynamic_provider();

            // Session
            let session_file = "session.json".to_string();
            let session_manager = SessionManager::new(&session_file);

            // Speaker
            let shared_speaker = Arc::new(Mutex::new(Speaker::new().unwrap_or_default()));

            // Tools
            let tools = Arc::new(ToolRegistry::default());
            // Register tools (SOTA set)
            tools.register_instance(WebSearchTool::new()).await;
            tools.register_instance(CodeExecTool::new()).await;
            tools.register_instance(MemoryQueryTool::new(memory.clone())).await;
            tools.register_instance(KnowledgeGraphTool::new(memory.clone())).await;
            tools.register_instance(ArtifactTool::default()).await;
            tools.register_instance(SandboxTool::default()).await;
            tools.register_instance(CodebaseTool::default()).await;
            tools.register_instance(ModelManager).await;
            tools.register_instance(SpeakerRsTool::new(shared_speaker.clone())).await;
            tools.register_instance(VisualizationTool::new()).await;
            tools.register_instance(ScienceTool::new()).await;
            tools.register_instance(VisionTool::new()).await;
            tools.register_instance(ForgeTool::new("custom_tools", tools.clone())).await;
            tools.register_instance(SystemTool::new(manager.clone())).await;

            let profile_manager = ProfileManager::new("agency_profile.json");
            let profile = profile_manager.load().await.unwrap_or_default();

            let mut supervisor = Supervisor::new_with_provider(provider.clone(), tools.clone())
                .with_memory(memory.clone())
                .with_session(session_manager)
                .with_episodic_memory(episodic_memory.clone())
                .with_profile(profile)
                .with_max_retries(2);

            let _ = supervisor.load_session().await;
            let shared_supervisor = Arc::new(Mutex::new(supervisor));

            // Manage State
            handle.manage(AgencyState {
                supervisor: shared_supervisor,
                speaker: shared_speaker,
                current_task: Arc::new(Mutex::new(None)),
                episodic_memory,
            });

            // EMBEDDED SERVICE: Listener (Whisper)
            // Must run in a dedicated thread due to cpal !Send constraints on macOS
            if std::env::var("AGENCY_ENABLE_EARS").unwrap_or_default() == "1" {
                std::thread::spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt.block_on(async {
                        if let Err(e) = rust_agency::services::listener::run_listener_server().await {
                            eprintln!("❌ Embedded Listener crashed: {}", e);
                        }
                    });
                });
            }

            // ──────────────────────────────────────────────────────────────────────────
            // SOVEREIGN ORGAN: Matrix Server (Conduit + Seatbelt)
            // ──────────────────────────────────────────────────────────────────────────
            if std::env::var("AGENCY_ENABLE_MATRIX").unwrap_or_default() == "1" {
                let app_handle_inner = handle.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri_plugin_shell::ShellExt;
                    
                    let data_dir = app_handle_inner.path().app_data_dir().unwrap().join("conduit");
                    let log_dir = app_handle_inner.path().app_log_dir().unwrap();
                    std::fs::create_dir_all(&data_dir).ok();
                    
                    // Note: We use sandbox-exec directly to wrap the sidecar
                    let sidecar_path = app_handle_inner.path().resolve("binaries/conduit", tauri::path::BaseDirectory::Resource).unwrap();
                    let sb_path = app_handle_inner.path().resolve("binaries/conduit.sb", tauri::path::BaseDirectory::Resource).unwrap();

                    println!("🔒 Launching Sandboxed Matrix Server...");
                    
                    let sidecar = app_handle_inner.shell().command("sandbox-exec")
                        .args([
                            "-f", &sb_path.to_string_lossy(),
                            "-D", &format!("DATA_DIR={}", data_dir.to_string_lossy()),
                            "-D", &format!("LOG_DIR={}", log_dir.to_string_lossy()),
                            &sidecar_path.to_string_lossy()
                        ])
                        .env("CONDUIT_DATABASE_PATH", data_dir.to_string_lossy().to_string())
                        .env("CONDUIT_PORT", "6167")
                        .env("CONDUIT_SERVER_NAME", "localhost");

                    match sidecar.spawn() {
                        Ok(_) => println!("✅ Matrix Organ (Conduit) is beating."),
                        Err(e) => eprintln!("❌ Matrix Organ failure: {}", e),
                    }
                });
            }
        });

        Ok(())
    })
    .invoke_handler(tauri::generate_handler![send_query, stop_inference, clear_memory])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}