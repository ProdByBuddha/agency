use anyhow::Result;
use std::io::stdout;
use std::time::{Duration, Instant};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};

use crate::orchestrator::{Supervisor, AgencyEvent, AGENCY_EVENT_BUS};
use crate::orchestrator::mvpk::Publication;

/// Events sent from the background worker or event bus to the TUI
enum AppEvent {
    Response(String, Option<Publication>),
    Error(String),
    SystemEvent(AgencyEvent),
}

/// TUI Application State
struct App {
    input: String,
    history: Vec<String>,
    logs: Vec<String>,
    status: String,
    is_orchestrating: bool,
    supervisor: Arc<Mutex<Supervisor>>,
    last_publication: Option<Publication>,
    speaker: Arc<Mutex<crate::orchestrator::Speaker>>,
    event_rx: mpsc::Receiver<AppEvent>,
    event_tx: mpsc::Sender<AppEvent>,
}

impl App {
    fn new(supervisor: Arc<Mutex<Supervisor>>, speaker: Arc<Mutex<crate::orchestrator::Speaker>>) -> Self {
        let (tx, rx) = mpsc::channel(100);
        
        // Subscribe to global agency events
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut bus_rx = AGENCY_EVENT_BUS.subscribe();
            while let Ok(event) = bus_rx.recv().await {
                let _ = tx_clone.send(AppEvent::SystemEvent(event)).await;
            }
        });

        Self {
            input: String::new(),
            history: Vec::new(),
            logs: Vec::new(),
            status: "Idle".to_string(),
            is_orchestrating: false,
            supervisor,
            last_publication: None,
            speaker,
            event_rx: rx,
            event_tx: tx,
        }
    }

    fn push_history(&mut self, msg: String) {
        self.history.push(msg);
        if self.history.len() > 50 { self.history.remove(0); }
    }

    fn push_log(&mut self, msg: String) {
        self.logs.push(msg);
        if self.logs.len() > 100 { self.logs.remove(0); }
    }

    async fn execute_query(&mut self, query: String) {
        self.is_orchestrating = true;
        self.status = "Orchestrating...".to_string();
        self.push_history(format!("λ User: {}", query));
        
        let supervisor = self.supervisor.clone();
        let tx = self.event_tx.clone();
        
        if query.starts_with("pai:") {
            let request = query.strip_prefix("pai:").unwrap().trim().to_string();
            tokio::spawn(async move {
                let mut pai = crate::orchestrator::pai::PAIOrchestrator::new(
                    pai_core::algorithm::EffortLevel::Standard,
                    supervisor.clone()
                );
                match pai.run_task(&request).await {
                    Ok(answer) => {
                        let _ = tx.send(AppEvent::Response(answer, None)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(e.to_string())).await;
                    }
                }
            });
            return;
        }

        if query.starts_with("/queue ") {
            let task_description = query.strip_prefix("/queue ").unwrap().trim().to_string();
            tokio::spawn(async move {
                let guard = supervisor.lock().await;
                let payload_json = serde_json::json!(task_description);
                match guard.schedule_task("autonomous_goal", payload_json).await {
                    Ok(id) => {
                        let _ = tx.send(AppEvent::Response(format!("Task Scheduled! ID: {}", id), None)).await;
                    },
                    Err(e) => {
                        let _ = tx.send(AppEvent::Error(format!("Failed to schedule task: {}", e))).await;
                    }
                }
            });
            return;
        }
        
        tokio::spawn(async move {
            let mut guard = supervisor.lock().await;
            match guard.handle(&query).await {
                Ok(result) => {
                    let answer = if let Some(ref p) = result.publication {
                        p.answer.clone()
                    } else {
                        result.answer.clone()
                    };
                    let _ = tx.send(AppEvent::Response(answer, result.publication)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(e.to_string())).await;
                }
            }
        });
    }

    async fn steer(&self, msg: String) {
        let guard = self.supervisor.lock().await;
        let _ = guard.steer(msg).await;
    }
}

pub struct AgencyCLI {
    supervisor: Arc<Mutex<Supervisor>>,
    speaker: Arc<Mutex<crate::orchestrator::Speaker>>,
}

impl AgencyCLI {
    pub fn new(supervisor: Arc<Mutex<Supervisor>>, speaker: Arc<Mutex<crate::orchestrator::Speaker>>) -> Self {
        Self { supervisor, speaker }
    }

    pub async fn run(self) -> Result<()> {
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        // Consuming self allows moving supervisor safely
        let mut app = App::new(self.supervisor, self.speaker.clone());

        let tick_rate = Duration::from_millis(50);
        let mut last_tick = Instant::now();

        loop {
            terminal.draw(|f| ui(f, &app))?;

            // Handle background events
            while let Ok(event) = app.event_rx.try_recv() {
                match event {
                    AppEvent::Response(answer, pub_obj) => {
                        app.push_history(format!("✅ Agency: {}", answer));
                        app.last_publication = pub_obj;
                        app.is_orchestrating = false;
                        app.status = "Idle".to_string();
                        
                        let speaker = app.speaker.clone();
                        tokio::spawn(async move {
                            let mut s = speaker.lock().await;
                            let _ = s.say(&answer.replace("*", "")).await;
                        });
                    }
                    AppEvent::Error(e) => {
                        app.push_history(format!("❌ Error: {}", e));
                        app.is_orchestrating = false;
                        app.status = "Error".to_string();
                    }
                    AppEvent::SystemEvent(e) => {
                        match e {
                            AgencyEvent::StatusUpdate(s) => app.status = s,
                            AgencyEvent::ToolCallStarted { tool } => app.push_log(format!("🔧 Tool Start: {}", tool)),
                            AgencyEvent::ToolCallFinished { tool, success } => {
                                let icon = if success { "✅" } else { "❌" };
                                app.push_log(format!("{} Tool End: {}", icon, tool));
                            }
                            AgencyEvent::TurnStarted { agent, model } => app.push_log(format!("🤖 Turn Start: {} ({})", agent, model)),
                            _ => app.push_log(format!("📝 Event: {:?}", e)),
                        }
                    }
                }
            }

            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Enter => {
                                let query = std::mem::take(&mut app.input);
                                if query == "quit" || query == "exit" {
                                    break;
                                }
                                if app.is_orchestrating {
                                    app.steer(query).await;
                                } else {
                                    app.execute_query(query).await;
                                }
                            }
                            KeyCode::Char(c) => {
                                app.input.push(c);
                            }
                            KeyCode::Backspace => {
                                app.input.pop();
                            }
                            KeyCode::Esc => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }

        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        Ok(())
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(chunks[0]);

    // History Area
    let history: Vec<ListItem> = app.history.iter().map(|msg| {
        ListItem::new(msg.as_str())
    }).collect();
    let history_list = List::new(history)
        .block(Block::default().borders(Borders::ALL).title(" 🏛️ FPF Interaction Trace "));
    f.render_widget(history_list, main_chunks[0]);

    // Sidebar: Telemetry & Logs
    let sidebar_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(60),
        ])
        .split(main_chunks[1]);

    // Telemetry
    let mut status_lines = vec![
        Line::from(vec![
            Span::styled("Status: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(&app.status, Style::default().fg(if app.is_orchestrating { Color::Yellow } else { Color::Green })),
        ]),
    ];
    
    if let Some(ref pub_obj) = app.last_publication {
        status_lines.push(Line::from(vec![
            Span::styled("Model: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::from(&pub_obj.telemetry.model),
        ]));
        status_lines.push(Line::from(vec![
            Span::styled("R-Score: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:.2}", pub_obj.reliability), Style::default().fg(Color::Cyan)),
        ]));
    }

    let status_para = Paragraph::new(status_lines)
        .block(Block::default().borders(Borders::ALL).title(" 📊 Telemetry "));
    f.render_widget(status_para, sidebar_chunks[0]);

    // PAI: Algorithm Progression Banner
    if app.is_orchestrating {
        let (r, g, b) = pai_core::visuals::VisualRenderer::get_phase_color(&pai_core::algorithm::AlgorithmPhase::Execute);
        let progress = pai_core::visuals::VisualRenderer::render_progress_bar(&pai_core::algorithm::AlgorithmPhase::Execute);
        let banner = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("THE ALGORITHM ", Style::default().add_modifier(Modifier::BOLD).fg(Color::Rgb(r, g, b))),
                Span::from(progress),
            ]),
            Line::from(vec![
                Span::styled("Phase: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("EXECUTE ", Style::default().fg(Color::Rgb(r, g, b))),
                Span::from("- Performing agent orchestration..."),
            ])
        ])
        .block(Block::default().borders(Borders::ALL).title(" ⚙️ PAI Engine "));
        
        // Render this below telemetry if orchestrating
        f.render_widget(banner, sidebar_chunks[1]);
    } else {
        // Logs
        let logs: Vec<ListItem> = app.logs.iter().map(|l| ListItem::new(l.as_str())).collect();
        let logs_list = List::new(logs)
            .block(Block::default().borders(Borders::ALL).title(" 📝 System Events "));
        f.render_widget(logs_list, sidebar_chunks[1]);
    }

    // Input Area
    let input_title = if app.is_orchestrating { " 🌀 Steering active agents... " } else { " λ Input " };
    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(if app.is_orchestrating { Color::Yellow } else { Color::Cyan }))
        .block(Block::default().borders(Borders::ALL).title(input_title));
    f.render_widget(input, chunks[1]);

    // Footer
    let help_text = format!(" ESC: Quit | /queue <goal>: Schedule Task | PID: {} | SOTA v0.2.0 ", std::process::id());
    let footer = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[2]);
}