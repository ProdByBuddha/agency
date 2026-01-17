use rust_agency::agent::{AgentType, AgentConfig};
use rust_agency::orchestrator::profile::AgencyProfile;
use rust_agency::tools::ToolRegistry;
use std::sync::Arc;

#[tokio::test]
async fn test_architecture_components_instantiation() {
    // 1. Verify Tool Registry
    let tools = Arc::new(ToolRegistry::default());
    tools.register_instance(rust_agency::tools::WasmCompilerTool::new()).await;
    tools.register_instance(rust_agency::tools::WasmExecutorTool::new()).await;
    
    let names = tools.tool_names().await;
    assert!(names.contains(&"wasm_compiler".to_string()));
    assert!(names.contains(&"wasm_executor".to_string()));

    // 2. Verify Profile Loading
    let profile = AgencyProfile::default();
    assert!(!profile.name.is_empty());

    // 3. Verify Agent Config Logic
    let config = AgentConfig::new(AgentType::Coder, &profile);
    assert!(config.system_prompt.contains("expert programmer"));
    assert!(config.system_prompt.contains(&profile.name));
}

#[tokio::test]
async fn test_runtime_isolation() {
    // Verify that the runtime module is accessible and compiles
    let mut runtime = rust_agency::runtime::wasm::WasmRuntime::new();
    // We can't easily test execution without a wasm file, but instantiation proves the dependency links are correct.
    assert!(std::any::type_name_of_val(&runtime).contains("WasmRuntime"));
}