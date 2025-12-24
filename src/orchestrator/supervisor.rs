use anyhow::Result;
use ollama_rs::Ollama;
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, info, warn};
use std::io::{self, Write};

use crate::agent::{
    AgentConfig, AgentResponse, AgentType, ReActAgent, ReActStep, Reflector,
    SimpleAgent, AutonomousMachine, BackgroundThoughtMachine, is_action_query,
    LLMCache, CachedProvider, OllamaProvider, LLMProvider, OpenAICompatibleProvider
};
use crate::memory::{EpisodicMemory, Memory, MemoryManager};
use crate::tools::{ToolRegistry, AgencyControlTool};

use super::{Plan, Planner, Router, SessionManager, profile::{AgencyProfile, ProfileManager}};

/// Result of supervisor execution
#[derive(Debug)]
pub struct SupervisorResult {
    /// The final answer
    pub answer: String,
    /// All agent responses from execution
    pub agent_responses: Vec<AgentResponse>,
    /// The plan that was executed (if any)
    pub plan: Option<Plan>,
    /// Whether execution was successful
    pub success: bool,
    /// Any reflections from failures
    pub reflections: Vec<String>,
}

/// Supervisor for multi-agent orchestration
pub struct Supervisor {
    ollama: Ollama,
    provider: Arc<dyn LLMProvider>,
    router: Router,
    planner: Planner,
    reflector: Reflector,
    tools: Arc<ToolRegistry>,
    memory: Option<Arc<dyn Memory>>,
    manager: Option<Arc<MemoryManager>>,
    session: Option<SessionManager>,
    profile_manager: Arc<ProfileManager>,
    profile: Arc<Mutex<AgencyProfile>>,
    episodic: EpisodicMemory,
    background_machine: Option<BackgroundThoughtMachine>,
    llm_cache: Arc<LLMCache>,
    concurrency_limit: Arc<Semaphore>,
    max_retries: usize,
}

impl Supervisor {
    pub fn new(ollama: Ollama, tools: Arc<ToolRegistry>) -> Self {
        let profile_manager = Arc::new(ProfileManager::new("agency_profile.json"));
        let profile = Arc::new(Mutex::new(AgencyProfile::default()));
        let llm_cache = Arc::new(LLMCache::new());
        let provider = Arc::new(CachedProvider::new(
            Arc::new(OllamaProvider::new(ollama.clone())),
            llm_cache.clone(),
        )) as Arc<dyn LLMProvider>;

        // Register the agency control tool immediately
        let control_tool = AgencyControlTool::new(profile_manager.clone(), profile.clone());
        let tools_clone = tools.clone();
        tokio::spawn(async move {
            tools_clone.register_instance(control_tool).await;
        });

        Self {
            ollama: ollama.clone(),
            provider: provider.clone(),
            router: Router::new(ollama.clone()).with_provider(provider.clone()),
            planner: Planner::new(ollama.clone()).with_provider(provider.clone()),
            reflector: Reflector::new(ollama).with_provider(provider),
            tools,
            memory: None,
            manager: None,
            session: None,
            profile_manager,
            profile,
            episodic: EpisodicMemory::default(),
            background_machine: None,
            llm_cache,
            concurrency_limit: Arc::new(Semaphore::new(2)),
            max_retries: 3,
        }
    }

    pub fn with_memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory.clone());
        self.manager = Some(Arc::new(MemoryManager::new(memory.clone())));
        self
    }

    pub async fn activate_background_thinking(&mut self) -> Result<()> {
        if let Some(ref memory) = self.memory {
            let profile = self.profile.lock().await.clone();
            let mut machine = BackgroundThoughtMachine::new(
                self.ollama.clone(),
                self.tools.clone(),
                memory.clone(),
                &profile,
            )
            .with_cache(self.llm_cache.clone());
            machine.start().await;
            self.background_machine = Some(machine);
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Memory must be initialized before activating background thinking"
            ))
        }
    }

    pub fn with_session(mut self, session: SessionManager) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_max_retries(mut self, retries: usize) -> Self {
        self.max_retries = retries;
        self
    }

    pub fn with_provider(mut self, provider: Arc<dyn LLMProvider>) -> Self {
        self.provider = provider.clone();
        self.router = self.router.with_provider(provider.clone());
        self.planner = self.planner.with_provider(provider.clone());
        self.reflector = self.reflector.with_provider(provider);
        self
    }

    pub fn with_provider_url(mut self, url: Option<String>) -> Self {
        if let Some(url_str) = url {
            let provider = Arc::new(OpenAICompatibleProvider::new(url_str, None));
            self.provider = provider.clone();
            self.router = self.router.with_provider(provider.clone());
            self.reflector = self.reflector.with_provider(provider);
        }
        self
    }

    fn create_cached_provider(&self) -> Arc<dyn LLMProvider> {
        Arc::new(CachedProvider::new(
            self.provider.clone(),
            self.llm_cache.clone(),
        ))
    }

    /// Load state from session file
    pub async fn load_session(&mut self) -> Result<()> {
        if let Some(ref session) = self.session {
            let state = session.load().await?;
            self.episodic = state.episodic_memory;
            info!("Restored {} turns from session history", self.episodic.turns().len());
        }
        // Also load profile
        let loaded_profile = self.profile_manager.load().await?;
        *self.profile.lock().await = loaded_profile;
        Ok(())
    }

    /// Clear conversation history and session
    pub async fn clear_history(&mut self) -> Result<()> {
        self.episodic.clear();
        if let Some(ref session) = self.session {
            session.clear().await?;
        }
        Ok(())
    }

    /// Get formatted conversation history
    pub fn conversation_history(&self) -> String {
        self.episodic.format_for_prompt()
    }

    /// Run in autonomous mode for a specific goal
    pub async fn run_autonomous(&mut self, goal: &str) -> Result<SupervisorResult> {
        let profile = self.profile.lock().await.clone();
        let mut machine = AutonomousMachine::new(self.ollama.clone(), self.tools.clone(), &profile, goal.to_string())
            .with_provider(self.create_cached_provider());
        
        info!("Starting autonomous thought loop for goal: {}", goal);
        let mut iterations = 0;
        let max_iters = 5;
        let mut all_responses: Vec<AgentResponse> = Vec::new();

        while iterations < max_iters {
            println!("\n🌀 Autonomous Thought Cycle {}...", iterations + 1);
            let response: AgentResponse = machine.run_iteration().await?;
            all_responses.push(response.clone());

            if response.success {
                return Ok(SupervisorResult {
                    answer: response.answer,
                    agent_responses: all_responses,
                    plan: None,
                    success: true,
                    reflections: vec![],
                });
            }
            iterations += 1;
        }

        Ok(SupervisorResult {
            answer: "Failed to achieve goal autonomously.".to_string(),
            agent_responses: all_responses,
            plan: None,
            success: false,
            reflections: vec!["Max autonomous iterations reached".to_string()],
        })
    }

    /// Handle a user query
    pub async fn handle(&mut self, query: &str) -> Result<SupervisorResult> {
        let start_handle = std::time::Instant::now();
        info!("Supervisor handling query: {}", query);
        
        // 1. Optimized tool loading: Only reload every 5 minutes
        static LAST_TOOL_RELOAD: Mutex<Option<std::time::Instant>> = Mutex::const_new(None);
        {
            let mut last_reload = LAST_TOOL_RELOAD.lock().await;
            if last_reload.is_none() || last_reload.unwrap().elapsed() > std::time::Duration::from_secs(300) {
                let _ = self.tools.load_dynamic_tools("custom_tools").await;
                *last_reload = Some(std::time::Instant::now());
            }
        }

        // 2. Optimized resource monitoring: Move to background to avoid blocking
        if let Some(ref manager) = self.manager {
            let manager_clone = manager.clone();
            tokio::spawn(async move {
                let _ = manager_clone.monitor_and_optimize().await;
            });
        }

        // Get current profile for prompts
        let current_profile = self.profile.lock().await.clone();

        // Add to episodic memory
        self.episodic.add_user(query);
        
        // Run routing and speculative memory search in parallel
        let query_owned = query.to_string();
        let router = self.router.clone();
        
        debug!("Starting parallel routing and memory search...");
        let routing_start = std::time::Instant::now();
        let routing_task = tokio::spawn(async move {
            router.route(&query_owned).await
        });

        // Fast heuristic to skip speculative memory search for very short/simple queries
        let is_simple_query = query.len() < 15 || 
            ["hi", "hello", "hey", "thanks", "ok", "yes", "no"].contains(&query.to_lowercase().as_str());

        let context_task = if self.memory.is_some() && !is_simple_query {
            let memory = self.memory.as_ref().unwrap().clone();
            let q_owned = query.to_string();
            Some(tokio::spawn(async move {
                let mem_start = std::time::Instant::now();
                match memory.search(&q_owned, 3).await {
                    Ok(entries) if !entries.is_empty() => {
                        let mut context_parts = Vec::new();
                        for e in entries {
                            let content = if e.content.len() > 1000 {
                                let mut end = 1000;
                                while !e.content.is_char_boundary(end) {
                                    end -= 1;
                                }
                                format!("{}... [truncated]", &e.content[..end])
                            } else {
                                e.content.clone()
                            };
                            context_parts.push(format!(
                                "[Memory {}]:\n{}\n",
                                e.timestamp.to_rfc3339(),
                                content
                            ));
                        }
                        debug!("Memory search took {:?}", mem_start.elapsed());
                        Some(context_parts.join("\n---\n"))
                    }
                    _ => None,
                }
            }))
        } else {
            None
        };

        // Wait for routing decision
        let routing_decision = routing_task.await??;
        debug!("Routing decision ({:?}) took {:?}", routing_decision.agent_type, routing_start.elapsed());

        // Get context from memory search ONLY if needed
        let memory_context = if routing_decision.should_search_memory {
            if let Some(task) = context_task {
                task.await?
            } else {
                None
            }
        } else {
            None
        };

        // Build overall context with pruning for performance
        let mut context = self.episodic.last_n(5) // Only last 5 turns for immediate context
            .iter()
            .map(|t| {
                let role = match t.role {
                    crate::memory::episodic::Role::User => "User",
                    crate::memory::episodic::Role::Assistant => "Assistant",
                    _ => "System",
                };
                format!("{}: {}", role, t.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        if routing_decision.should_search_memory {
            if let Some(mem_ctx) = memory_context {
                context.push_str("\n\n## Relevant Past Information (Summary)\n");
                // Only take first 1000 chars of memory context to avoid bloating
                let pruned_mem = if mem_ctx.len() > 1000 {
                    crate::agent::truncate(&mem_ctx, 1000)
                } else {
                    mem_ctx
                };
                context.push_str(&pruned_mem);
            }
        }

        let mut agent_responses = Vec::new();
        let mut final_answer = String::new();
        let mut plan_executed = None;
        let mut overall_success = true;
        let mut reflections = Vec::new();

        // Check if we should use planning
        if routing_decision.agent_type == AgentType::Planner && !self.planner.should_skip_planning(query) {
            debug!("Starting planning phase...");
            let plan_start = std::time::Instant::now();
            let plan = self.planner.decompose(query).await?;
            debug!("Decomposition into {} steps took {:?}", plan.steps.len(), plan_start.elapsed());
            
            let mut current_plan = plan;
            let execution_context = Arc::new(tokio::sync::RwLock::new(context.clone()));

            while !current_plan.is_complete {
                let ready_steps: Vec<_> = current_plan.ready_steps().into_iter().cloned().collect();
                if ready_steps.is_empty() { break; }

                info!("Parallel executing {} ready steps...", ready_steps.len());
                let steps_exec_start = std::time::Instant::now();
                
                let mut step_futures: Vec<tokio::task::JoinHandle<Result<(usize, Result<AgentResponse, String>), anyhow::Error>>> = Vec::new();
                for step in ready_steps {
                    let ollama = self.ollama.clone();
                    let tools = self.tools.clone();
                    let memory = self.memory.clone();
                    let provider = self.create_cached_provider();
                    let ctx_clone = execution_context.clone();
                    let step_desc = step.description.clone();
                    let agent_type = step.agent_type;
                    let profile_clone = current_profile.clone();
                    let semaphore = self.concurrency_limit.clone();

                    step_futures.push(tokio::spawn(async move {
                        // Wait for a permit before starting agent execution
                        let _permit = semaphore.acquire().await.map_err(|e| anyhow::anyhow!("Semaphore error: {}", e))?;
                        
                        let config = AgentConfig::new(agent_type, &profile_clone);
                        let mut agent = ReActAgent::new(ollama.clone(), config, tools.clone())
                            .with_provider(provider.clone());
                        if let Some(m) = memory { agent = agent.with_memory(m); }
                        
                        let ctx = ctx_clone.read().await.clone();
                        let mut steps = Vec::new();
                        let mut iteration = 0;
                        let max_iters = 5;
                        let mut final_res: Option<AgentResponse> = None;

                        while iteration < max_iters {
                            let s = agent.step(&step_desc, &steps, Some(&ctx)).await
                                .map_err(|e| anyhow::anyhow!("Step failed: {}", e))?;
                            
                            if s.is_final {
                                let answer = s.answer.clone().unwrap_or_else(|| s.thought.clone());
                                steps.push(s);
                                final_res = Some(AgentResponse::success(answer, steps.clone(), agent_type));
                                break;
                            }

                            if !s.actions.is_empty() {
                                let mut observations = Vec::new();
                                for action in &s.actions {
                                    let tool = tools.get_tool(&action.name).await;
                                    let needs_confirm = tool.as_ref().map(|t| t.requires_confirmation()).unwrap_or(false);

                                    let proceed = if needs_confirm {
                                        println!("\n🛡️  PERMISSION REQUEST (Step {})", step.step_num);
                                        println!("   Agent wants to use '{}'", action.name);
                                        println!("   Parameters: {}", serde_json::to_string_pretty(&action.parameters).unwrap_or_default());
                                        print!("   Allow? [y/N]: ");
                                        io::stdout().flush()?;
                                        let mut input = String::new();
                                        io::stdin().read_line(&mut input)?;
                                        input.trim().to_lowercase() == "y"
                                    } else {
                                        true
                                    };

                                    if proceed {
                                        let res = tools.execute(action).await;
                                        observations.push(match res {
                                            Ok(o) => o.summary,
                                            Err(e) => format!("Tool execution failed: {}", e),
                                        });
                                    } else {
                                        observations.push("USER DENIED PERMISSION: This action was blocked by the human supervisor.".to_string());
                                    }
                                }
                                steps.push(ReActStep {
                                    thought: s.thought.clone(),
                                    actions: s.actions.clone(),
                                    observations,
                                    is_final: false,
                                    answer: None,
                                });
                            } else {
                                steps.push(s);
                            }
                            iteration += 1;
                        }

                        let response = final_res.unwrap_or_else(|| AgentResponse::failure("Max iterations reached", steps, agent_type));
                        
                        if response.success {
                            // Unified dual-consensus review for steps as well
                            let r1_reflector = Reflector::new(ollama.clone()).with_provider(provider.clone()).with_model("deepseek-r1:8b");
                            let qwen_reflector = Reflector::new(ollama.clone()).with_provider(provider.clone()).with_model("qwen2.5-coder:7b");
                            
                            let r1_rev = r1_reflector.review_response(&step_desc, &response.answer, &response.steps).await;
                            let qwen_rev = qwen_reflector.review_response(&step_desc, &response.answer, &response.steps).await;
                            
                            let should_retry = match (&r1_rev, &qwen_rev) {
                                (Ok(r1), Ok(q)) => r1.should_retry || q.should_retry,
                                (Ok(r), _) => r.should_retry,
                                (_, Ok(q)) => q.should_retry,
                                _ => false
                            };

                            if should_retry {
                                return Ok((step.step_num, Err(format!("Step review failed: Consensus rejection"))));
                            }
                        }
                        
                        Ok((step.step_num, Ok(response)))
                    }));
                }

                let results = futures_util::future::join_all(step_futures).await;
                debug!("Parallel execution of ready steps took {:?}", steps_exec_start.elapsed());
                let mut step_failed = false;

                for res in results {
                    let (step_num, step_res) = res??;
                    match step_res {
                        Ok(response) if response.success => {
                            let output = response.answer.clone();
                            current_plan.complete_step(step_num, &output);
                            
                            // Update shared context
                            let mut ctx = execution_context.write().await;
                            ctx.push_str(&format!("\n\nStep {} Result: {}", step_num, output));
                            
                            agent_responses.push(response);
                        }
                        Ok(response) => {
                            let err_msg = response.error.clone().unwrap_or_else(|| "Unknown error".to_string());
                            warn!("Step {} agent execution failed: {}", step_num, err_msg);
                            step_failed = true;
                            overall_success = false;
                            final_answer = format!("Step {} failed: {}", step_num, err_msg);
                            agent_responses.push(response);
                            break;
                        }
                        Err(e) => {
                            warn!("Step {} failed in parallel execution (Review or Runtime): {}", step_num, e);
                            step_failed = true;
                            overall_success = false;
                            final_answer = format!("Task failed at step {}: {}", step_num, e);
                            break;
                        }
                    }
                }

                if step_failed { break; }
            }

            if overall_success {
                final_answer = current_plan.steps.last().and_then(|s| s.output.clone()).unwrap_or_else(|| "Plan completed successfully.".to_string());
                plan_executed = Some(current_plan);
            }
        } else {
            // Single agent execution
            let single_exec_start = std::time::Instant::now();
            let config = AgentConfig::new(routing_decision.agent_type, &current_profile);
            let provider = self.create_cached_provider();
            
            let (final_ans, final_success, final_agent_res) = if routing_decision.agent_type == AgentType::GeneralChat {
                let agent = SimpleAgent::new(self.ollama.clone(), config)
                    .with_provider(provider);
                let res = agent.execute_simple(query, Some(&context)).await?;
                (res.answer.clone(), res.success, res)
            } else {
                let mut agent = ReActAgent::new(self.ollama.clone(), config, self.tools.clone())
                    .with_provider(provider.clone());
                if let Some(ref memory) = self.memory { agent = agent.with_memory(memory.clone()); }

                let mut steps = Vec::new();
                let mut attempts = 0;
                let mut final_agent_response: Option<AgentResponse> = None;

                while attempts < self.max_retries {
                    let mut iteration = 0;
                    let max_iters = 5; 
                    let mut current_agent_response: Option<AgentResponse> = None;

                    while iteration < max_iters {
                        let step_start = std::time::Instant::now();
                        let mut step = match agent.step(query, &steps, Some(&context)).await {
                            Ok(s) => s,
                            Err(e) => {
                                current_agent_response = Some(AgentResponse::failure(e.to_string(), steps.clone(), routing_decision.agent_type));
                                break;
                            }
                        };
                        debug!("ReAct iteration {} step took {:?}", iteration + 1, step_start.elapsed());

                        // LAZINESS FILTER: Detect finishing without action for complex queries
                        if step.is_final && steps.is_empty() && is_action_query(query) {
                            warn!("Laziness detected: Agent tried to finish without any tool calls for an action query.");
                            let hint = "SYSTEM HINT: Your query requires ACTION (creating, analyzing, searching). You MUST use tools first. Do NOT provide a final answer until you have observations from the required tools (e.g., forge_tool, code_exec, codebase_explorer).";
                            
                            step.is_final = false;
                            step.thought = format!("{} [REJECTED: No tool used. I must use tools.]", step.thought);
                            
                            let mut hint_step = step.clone();
                            hint_step.observations.push(hint.to_string());
                            steps.push(hint_step);
                            continue;
                        }

                        if step.is_final {
                            let answer = step.answer.clone().unwrap_or_else(|| step.thought.clone());
                            steps.push(step);
                            current_agent_response = Some(AgentResponse::success(answer, steps.clone(), routing_decision.agent_type));
                            break;
                        }

                        if !step.actions.is_empty() {
                            let mut observations = Vec::new();
                            for action in &step.actions {
                                let tool = self.tools.get_tool(&action.name).await;
                                let needs_confirm = tool.as_ref().map(|t| t.requires_confirmation()).unwrap_or(false);

                                let proceed = if needs_confirm {
                                    println!("\n🛡️  PERMISSION REQUEST: Agent wants to use '{}'", action.name);
                                    println!("   Parameters: {}", serde_json::to_string_pretty(&action.parameters).unwrap_or_default());
                                    print!("   Allow? [y/N]: ");
                                    io::stdout().flush()?;
                                    let mut input = String::new();
                                    io::stdin().read_line(&mut input)?;
                                    input.trim().to_lowercase() == "y"
                                } else {
                                    true
                                };

                                if proceed {
                                    let tool_start = std::time::Instant::now();
                                    let res = self.tools.execute(action).await;
                                    debug!("Tool '{}' execution took {:?}", action.name, tool_start.elapsed());
                                    observations.push(match res {
                                        Ok(o) => o.summary,
                                        Err(e) => format!("Tool execution failed: {}", e),
                                    });
                                } else {
                                    info!("User denied permission for tool: {}", action.name);
                                    observations.push("USER DENIED PERMISSION: This action was blocked by the human supervisor. Try a different approach.".to_string());
                                }
                            }
                            steps.push(ReActStep {
                                thought: step.thought.clone(),
                                actions: step.actions.clone(),
                                observations,
                                is_final: false,
                                answer: None,
                            });
                        } else {
                            steps.push(step);
                        }
                        iteration += 1;
                    }

                    let response = current_agent_response.unwrap_or_else(|| {
                        AgentResponse::failure("Max iterations reached", steps.clone(), routing_decision.agent_type)
                    });

                    if !response.success {
                        attempts += 1;
                        let reflection_start = std::time::Instant::now();
                        let reflection_res = self.reflector.analyze_failure(query, &response.steps, response.error.as_deref()).await?;
                        debug!("Failure reflection took {:?}", reflection_start.elapsed());
                        
                        let reflection = reflection_res.analysis.clone();
                        reflections.push(reflection.clone());
                        if !reflection_res.should_retry { 
                            final_agent_response = Some(response);
                            break; 
                        }
                        info!("Retry attempt {} with failure reflection", attempts);
                    } else if routing_decision.agent_type == AgentType::GeneralChat {
                        // Skip review for general chat to avoid over-analyzing greetings
                        final_agent_response = Some(response);
                        break;
                    } else {
                        info!("Running dual-model consensus review (DeepSeek + Qwen)...");
                        let r1_reflector = Reflector::new(self.ollama.clone()).with_provider(self.provider.clone()).with_model("deepseek-r1:8b");
                        let qwen_reflector = Reflector::new(self.ollama.clone()).with_provider(self.provider.clone()).with_model("qwen2.5-coder:7b");
                        
                        let rev1 = tokio::time::timeout(
                            std::time::Duration::from_secs(120), 
                            r1_reflector.review_response(query, &response.answer, &response.steps)
                        ).await.ok().and_then(|r| r.ok());

                        let rev2 = tokio::time::timeout(
                            std::time::Duration::from_secs(120), 
                            qwen_reflector.review_response(query, &response.answer, &response.steps)
                        ).await.ok().and_then(|r| r.ok());

                        let should_retry = rev1.as_ref().map(|r| r.should_retry).unwrap_or(false) 
                                        || rev2.as_ref().map(|r| r.should_retry).unwrap_or(false);

                        if should_retry {
                            attempts += 1;
                            let analysis1 = rev1.map(|r| r.analysis).unwrap_or_else(|| "Llama Timeout".to_string());
                            let analysis2 = rev2.map(|r| r.analysis).unwrap_or_else(|| "Qwen Timeout".to_string());
                            let reflection = format!("CRITICAL REVIEW FINDING: Previous response rejected.\nLlama: {}\nQwen: {}", analysis1, analysis2);
                            reflections.push(format!("Consensus review finding: {}", reflection));
                            
                            if attempts >= self.max_retries {
                                info!("Max retries reached after consensus rejection.");
                                final_agent_response = Some(AgentResponse::failure(
                                    format!("Consensus review failed after {} attempts. Last reason: {}", self.max_retries, reflection),
                                    response.steps,
                                    routing_decision.agent_type
                                ));
                                break;
                            }
                            
                            info!("Retry attempt {} with consensus review reflection", attempts);
                            // Reset steps for a clean retry with the reflection in context
                            context.push_str(&format!("\n\n## Feedback from Previous Attempt\n{}", reflection));
                            steps = Vec::new();
                        } else {
                            info!("Consensus review passed.");
                            final_agent_response = Some(response);
                            break; 
                        }
                    }
                }

                let final_res = final_agent_response.unwrap_or_else(|| AgentResponse::failure("Failed after retries", steps, routing_decision.agent_type));
                (final_res.answer.clone(), final_res.success, final_res)
            };

            final_answer = final_ans;
            overall_success = final_success;
            agent_responses.push(final_agent_res);
            debug!("Single agent execution took {:?}", single_exec_start.elapsed());
        }

        // Add assistant response to episodic memory
        self.episodic.add_assistant(&final_answer, None);

        // Periodic Memory Consolidation
        if self.episodic.len() >= 10 && self.manager.is_some() {
            let manager = self.manager.as_ref().unwrap().clone();
            let ollama = self.ollama.clone();
            let episodic = self.episodic.clone();
            tokio::spawn(async move { let _ = manager.distill_and_consolidate(&ollama, &episodic).await; });
        }

        // Save session if enabled
        if let Some(ref session) = self.session { let _ = session.save(&self.episodic, plan_executed.as_ref()).await; }

        info!("Total query handling took {:?}", start_handle.elapsed());
        debug!("Final Answer DEBUG: {}", final_answer);
        Ok(SupervisorResult {
            answer: final_answer,
            agent_responses,
            plan: plan_executed,
            success: overall_success,
            reflections,
        })
    }
}