use std::io::stdout;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use openagents_core::{CatalogItemKind, ToolKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::catalog::{CatalogItem, curated_items};
use crate::control::ControlPlane;
use crate::detection::DetectionReport;
use crate::runtime::{self, SyncSummary};
use crate::setup::{MemoryBackendPreset, ProfilePreset, SetupSelection};

const BOOT_TICKS: u16 = 10;
const TEAL: Color = Color::Rgb(79, 212, 201);
const LIME: Color = Color::Rgb(185, 255, 102);
const SLATE: Color = Color::Rgb(123, 151, 166);
const IVORY: Color = Color::Rgb(223, 237, 232);
const CHARCOAL: Color = Color::Rgb(22, 28, 37);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum SetupScreen {
    Boot,
    Detection,
    AskProfile,
    AskMemory,
    AskTools,
    AskSkills,
    AskMcps,
    Confirm,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeroState {
    Scanning,
    Listening,
    Ready,
}

pub struct SetupApp {
    pub report: DetectionReport,
    pub selection: SetupSelection,
    pub screen: SetupScreen,
    pub tool_cursor: usize,
    pub skill_cursor: usize,
    pub mcp_cursor: usize,
    pub boot_tick: u16,
    pub status: String,
    pub completion: Option<SyncSummary>,
}

impl SetupApp {
    pub fn new(report: DetectionReport, selection: SetupSelection) -> Self {
        Self {
            report,
            selection,
            screen: SetupScreen::Boot,
            tool_cursor: 0,
            skill_cursor: 0,
            mcp_cursor: 0,
            boot_tick: 0,
            status: boot_loading_message(0),
            completion: None,
        }
    }

    pub fn advance_boot(&mut self) {
        if self.screen != SetupScreen::Boot {
            return;
        }
        self.boot_tick = self.boot_tick.saturating_add(1);
        self.status = boot_loading_message(self.boot_tick.into());
        if self.boot_tick >= BOOT_TICKS {
            self.screen = SetupScreen::Detection;
            self.status = screen_status(self.screen).to_string();
        }
    }

    pub fn advance_conversation(&mut self) {
        self.screen = match self.screen {
            SetupScreen::Boot => SetupScreen::Detection,
            SetupScreen::Detection => SetupScreen::AskProfile,
            SetupScreen::AskProfile => SetupScreen::AskMemory,
            SetupScreen::AskMemory => SetupScreen::AskTools,
            SetupScreen::AskTools => SetupScreen::AskSkills,
            SetupScreen::AskSkills => SetupScreen::AskMcps,
            SetupScreen::AskMcps => SetupScreen::Confirm,
            SetupScreen::Confirm | SetupScreen::Complete => self.screen,
        };
        self.status = screen_status(self.screen).to_string();
    }

    pub fn previous_screen(&mut self) {
        self.screen = match self.screen {
            SetupScreen::AskMemory => SetupScreen::AskProfile,
            SetupScreen::AskTools => SetupScreen::AskMemory,
            SetupScreen::AskSkills => SetupScreen::AskTools,
            SetupScreen::AskMcps => SetupScreen::AskSkills,
            SetupScreen::Confirm => SetupScreen::AskMcps,
            other => other,
        };
        self.status = screen_status(self.screen).to_string();
    }

    pub fn cycle_profile(&mut self, forward: bool) {
        self.selection.profile_preset = if forward {
            self.selection.profile_preset.next()
        } else {
            self.selection.profile_preset.previous()
        };
        crate::setup::refresh_catalog_recommendations(&mut self.selection);
    }

    pub fn cycle_memory(&mut self, forward: bool) {
        self.selection.memory_backend = if forward {
            self.selection.memory_backend.next()
        } else {
            self.selection.memory_backend.previous()
        };
        crate::setup::refresh_catalog_recommendations(&mut self.selection);
    }

    pub fn move_tool_cursor(&mut self, forward: bool) {
        let len = tool_order().len();
        self.tool_cursor = if forward {
            (self.tool_cursor + 1) % len
        } else if self.tool_cursor == 0 {
            len - 1
        } else {
            self.tool_cursor - 1
        };
    }

    pub fn toggle_current_tool(&mut self) {
        let tool = tool_order()[self.tool_cursor];
        if let Some(index) = self
            .selection
            .enabled_tools
            .iter()
            .position(|item| *item == tool)
        {
            self.selection.enabled_tools.remove(index);
        } else {
            self.selection.enabled_tools.push(tool);
            self.selection.enabled_tools.sort();
        }
    }

    pub fn move_catalog_cursor(&mut self, kind: CatalogItemKind, forward: bool) {
        let len = match kind {
            CatalogItemKind::Skill => skill_catalog().len(),
            CatalogItemKind::Mcp => mcp_catalog().len(),
        };
        let cursor = match kind {
            CatalogItemKind::Skill => &mut self.skill_cursor,
            CatalogItemKind::Mcp => &mut self.mcp_cursor,
        };
        *cursor = if forward {
            (*cursor + 1) % len
        } else if *cursor == 0 {
            len - 1
        } else {
            *cursor - 1
        };
    }

    pub fn toggle_current_catalog_item(&mut self, kind: CatalogItemKind) {
        let item_id = match kind {
            CatalogItemKind::Skill => skill_catalog()[self.skill_cursor].id,
            CatalogItemKind::Mcp => mcp_catalog()[self.mcp_cursor].id,
        }
        .to_string();

        let target = match kind {
            CatalogItemKind::Skill => &mut self.selection.selected_skills,
            CatalogItemKind::Mcp => &mut self.selection.selected_mcp_servers,
        };
        if let Some(index) = target.iter().position(|item| item == &item_id) {
            target.remove(index);
        } else {
            target.push(item_id);
            target.sort();
            target.dedup();
        }
    }
}

pub fn run_tui(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
) -> Result<()> {
    match ControlPlane::load(config_override, manifest_override) {
        Ok(control) => run_dashboard(&control, cwd),
        Err(_) => run_setup(config_override, manifest_override, cwd, false),
    }
}

pub fn run_setup(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
    dry_run: bool,
) -> Result<()> {
    let (report, selection) =
        runtime::recommended_setup_selection(config_override, manifest_override, cwd)?;

    if dry_run {
        let config = crate::setup::selection_to_config(&selection);
        println!("{}", serde_yaml::to_string(&config)?);
        return Ok(());
    }

    enable_raw_mode().context("failed to enable raw mode")?;
    let mut terminal = setup_terminal()?;
    let mut app = SetupApp::new(report, selection);
    let result = setup_loop(&mut terminal, &mut app, config_override, cwd);
    teardown_terminal(&mut terminal)?;
    result
}

fn setup_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut SetupApp,
    config_override: Option<&Path>,
    cwd: &Path,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw_setup(frame, app))?;

        if event::poll(Duration::from_millis(180))? {
            if let Event::Key(key) = event::read()? {
                match app.screen {
                    SetupScreen::Boot => match key.code {
                        KeyCode::Enter => {
                            app.screen = SetupScreen::Detection;
                            app.status = screen_status(app.screen).to_string();
                        }
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::Detection => match key.code {
                        KeyCode::Enter => app.advance_conversation(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::AskProfile => match key.code {
                        KeyCode::Left => app.cycle_profile(false),
                        KeyCode::Right => app.cycle_profile(true),
                        KeyCode::Enter => app.advance_conversation(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::AskMemory => match key.code {
                        KeyCode::Left => app.cycle_memory(false),
                        KeyCode::Right => app.cycle_memory(true),
                        KeyCode::Enter => app.advance_conversation(),
                        KeyCode::Backspace => app.previous_screen(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::AskTools => match key.code {
                        KeyCode::Up => app.move_tool_cursor(false),
                        KeyCode::Down => app.move_tool_cursor(true),
                        KeyCode::Char(' ') => app.toggle_current_tool(),
                        KeyCode::Enter => app.advance_conversation(),
                        KeyCode::Backspace => app.previous_screen(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::AskSkills => match key.code {
                        KeyCode::Up => app.move_catalog_cursor(CatalogItemKind::Skill, false),
                        KeyCode::Down => app.move_catalog_cursor(CatalogItemKind::Skill, true),
                        KeyCode::Char(' ') => {
                            app.toggle_current_catalog_item(CatalogItemKind::Skill)
                        }
                        KeyCode::Enter => app.advance_conversation(),
                        KeyCode::Backspace => app.previous_screen(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::AskMcps => match key.code {
                        KeyCode::Up => app.move_catalog_cursor(CatalogItemKind::Mcp, false),
                        KeyCode::Down => app.move_catalog_cursor(CatalogItemKind::Mcp, true),
                        KeyCode::Char(' ') => app.toggle_current_catalog_item(CatalogItemKind::Mcp),
                        KeyCode::Enter => app.advance_conversation(),
                        KeyCode::Backspace => app.previous_screen(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::Confirm => match key.code {
                        KeyCode::Enter => {
                            match runtime::apply_setup(config_override, cwd, &app.selection) {
                                Ok(summary) => {
                                    app.completion = Some(summary);
                                    app.screen = SetupScreen::Complete;
                                    app.status = screen_status(app.screen).to_string();
                                }
                                Err(error) => {
                                    app.status = format!("I could not finish setup: {error}")
                                }
                            }
                        }
                        KeyCode::Backspace => app.previous_screen(),
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                    SetupScreen::Complete => match key.code {
                        KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        _ => {}
                    },
                }
            }
        } else if app.screen == SetupScreen::Boot {
            app.advance_boot();
        }
    }
}

fn run_dashboard(control: &ControlPlane, cwd: &Path) -> Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut terminal = setup_terminal()?;
    let active_profile = control.active_profile_name(cwd, None);
    let resolved = control.resolved_profile(&active_profile)?;
    let report = runtime::load_detection_report()?;

    let result = loop {
        terminal.draw(|frame| draw_dashboard(frame, control, cwd, &resolved, &report))?;
        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(key) = event::read()? {
                if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    break Ok(());
                }
            }
        }
    };
    teardown_terminal(&mut terminal)?;
    result
}

fn draw_dashboard(
    frame: &mut ratatui::Frame<'_>,
    control: &ControlPlane,
    cwd: &Path,
    profile: &openagents_core::ResolvedProfile,
    report: &DetectionReport,
) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(40), Constraint::Min(10)])
        .split(chunks[1]);
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    frame.render_widget(
        Paragraph::new(hero_lines(HeroState::Ready, "OpenAgents Control Center"))
            .block(panel("OpenAgents"))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(vec![
            line_plain(format!("workspace  {}", control.config.workspace_name)),
            line_plain(format!("device     {}", control.overlay.device_name)),
            line_plain(format!(
                "project    {}",
                cwd.file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("current-folder")
            )),
            line_plain(""),
            line_accent(format!("profile    {}", profile.name)),
            line_plain(format!("memory     {}", profile.memory.provider)),
            line_plain(format!(
                "tools      {}",
                profile
                    .tools
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        ])
        .block(panel("Session")),
        top[0],
    );

    frame.render_widget(
        Paragraph::new(vec![
            line_plain("One global control plane now drives the desired state for this device and this project attachment."),
            line_plain("Run `openagents-kit setup` when you want the guided conversation again."),
            line_plain(""),
            line_accent(format!("config root  {}", control.root.display())),
            line_accent(format!("managed root {}", control.managed_root().display())),
            line_accent(format!(
                "attachment   {}",
                control
                    .attached_profile_for(cwd)
                    .unwrap_or_else(|| "default-profile".to_string())
            )),
        ])
        .block(panel("Status")),
        top[1],
    );

    frame.render_widget(
        Paragraph::new(vec![
            line_plain("Desired Capabilities"),
            line_plain(""),
            line_plain(format!("skills: {}", join_or_none(&profile.skills))),
            line_plain(format!(
                "mcp servers: {}",
                join_or_none(&profile.mcp_servers)
            )),
            line_plain(""),
            line_plain("Local Discovery"),
            line_plain(format!(
                "tools seen: {}",
                report
                    .detections
                    .iter()
                    .map(|item| item.tool.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            line_plain(format!(
                "skills seen: {}",
                join_or_none(&report.installed_skills)
            )),
            line_plain(format!(
                "mcp seen: {}",
                join_or_none(&report.installed_mcp_servers)
            )),
        ])
        .block(panel("Health")),
        middle[0],
    );

    frame.render_widget(
        Paragraph::new(vec![
            line_plain("Next Actions"),
            line_plain(""),
            line_accent("openagents-kit sync"),
            line_plain("Reconcile the global desired state into managed tool outputs."),
            line_plain(""),
            line_accent("openagents-kit doctor"),
            line_plain("Check drift, missing tools, missing skills, and missing MCP servers."),
            line_plain(""),
            line_accent("openagents-kit setup"),
            line_plain(
                "Run the assistant conversation again if you want to change the control plane.",
            ),
        ])
        .block(panel("Guide")),
        middle[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("q", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" exit  ", Style::default().fg(SLATE)),
            Span::styled("openagents-kit setup", Style::default().fg(TEAL)),
            Span::styled(" rerun the assistant interview", Style::default().fg(SLATE)),
        ]))
        .block(panel("Controls")),
        chunks[3],
    );
}

fn draw_setup(frame: &mut ratatui::Frame<'_>, app: &SetupApp) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Length(match app.screen {
                SetupScreen::Boot | SetupScreen::Detection | SetupScreen::Complete => 8,
                SetupScreen::Confirm => 10,
                _ => 11,
            }),
            Constraint::Length(3),
        ])
        .split(area);

    let hero_state = match app.screen {
        SetupScreen::Boot => HeroState::Scanning,
        SetupScreen::Complete => HeroState::Ready,
        _ => HeroState::Listening,
    };

    frame.render_widget(
        Paragraph::new(hero_lines(hero_state, hero_title(app.screen)))
            .block(panel("Welcome"))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(conversation_lines(app))
            .block(panel("Conversation"))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    match app.screen {
        SetupScreen::Boot
        | SetupScreen::Detection
        | SetupScreen::Confirm
        | SetupScreen::Complete => {
            frame.render_widget(
                Paragraph::new(option_lines(app))
                    .block(panel(option_title(app.screen)))
                    .wrap(Wrap { trim: false }),
                chunks[2],
            );
        }
        _ => {
            frame.render_widget(
                List::new(option_items(app))
                    .block(panel(option_title(app.screen)))
                    .highlight_style(Style::default().fg(LIME)),
                chunks[2],
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(setup_controls(app.screen), Style::default().fg(SLATE)),
            Span::raw("  "),
            Span::styled(&app.status, Style::default().fg(TEAL)),
        ]))
        .block(panel("Controls")),
        chunks[3],
    );
}

fn hero_title(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "OpenAgents is checking this device",
        SetupScreen::Detection => "I found the starting point for your control plane",
        SetupScreen::AskProfile => "Let me shape the profile first",
        SetupScreen::AskMemory => "Now I need a memory layer",
        SetupScreen::AskTools => "Next I will manage your tools",
        SetupScreen::AskSkills => "These are the skills I suggest",
        SetupScreen::AskMcps => "These are the MCP servers I suggest",
        SetupScreen::Confirm => "I am ready to write and sync your control plane",
        SetupScreen::Complete => "Your global OpenAgents control plane is ready",
    }
}

fn hero_lines(state: HeroState, title: &str) -> Vec<Line<'static>> {
    let face = match state {
        HeroState::Scanning => "[]  []  ..",
        HeroState::Listening => "[]  []  --",
        HeroState::Ready => "[]  []  ^^",
    };

    vec![
        line_accent(format!("OpenAgents  {}", title)),
        line_plain(""),
        line_plain("                  ________________________"),
        line_plain(format!("              .--|  {face}              |--.")),
        line_plain("             /___|      .----.          |___\\"),
        line_plain("             |   |     /|_||_|\\         |   |"),
        line_plain("             |[] |     ||____||         | []|"),
        line_plain("             |___|_____|/____\\|_________|___|"),
        line_plain("                /_/        /__\\        \\_\\"),
    ]
}

fn conversation_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let mut lines = vec![
        line_assistant(
            "Welcome. I’m going to set up one OpenAgents control plane for your tools, skills, MCP servers, and memory.",
        ),
        line_assistant(format!(
            "I checked this device and found {} supported tools.",
            app.report.detections.len()
        )),
    ];

    if app.report.detections.is_empty() {
        lines.push(line_assistant(
            "I did not find a strong existing tool footprint, so I prepared a safe starter setup.",
        ));
    } else {
        for detection in &app.report.detections {
            lines.push(line_assistant(format!(
                "I found {} from {}.",
                detection.summary,
                detection.evidence_path.display()
            )));
        }
    }

    if !app.report.has_memory_layer {
        lines.push(line_assistant(
            "I do not see a consistent memory layer yet, so I will recommend a local one first.",
        ));
    }

    if app.screen >= SetupScreen::AskProfile {
        lines.push(line_user(format!(
            "Keep profile: {}",
            app.selection.profile_preset.label()
        )));
    }
    if app.screen >= SetupScreen::AskMemory {
        lines.push(line_user(format!(
            "Use memory: {}",
            app.selection.memory_backend.label()
        )));
    }
    if app.screen >= SetupScreen::AskTools {
        lines.push(line_user(format!(
            "Manage tools: {}",
            join_or_none(
                &app.selection
                    .enabled_tools
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            )
        )));
    }
    if app.screen >= SetupScreen::AskSkills {
        lines.push(line_user(format!(
            "Install skills: {}",
            join_or_none(&app.selection.selected_skills)
        )));
    }
    if app.screen >= SetupScreen::AskMcps {
        lines.push(line_user(format!(
            "Install MCP servers: {}",
            join_or_none(&app.selection.selected_mcp_servers)
        )));
    }

    lines.push(line_plain(""));
    lines.extend(current_prompt_lines(app));
    lines
}

fn current_prompt_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![line_assistant(
            "I’m still scanning. Press Enter if you want me to skip the rest of the boot animation.",
        )],
        SetupScreen::Detection => vec![
            line_assistant("Here is my recommendation."),
            line_assistant(format!(
                "I’ll create a global control plane called `{}` and attach this project to `{}`.",
                app.selection.workspace_name,
                app.selection.profile_preset.profile_name()
            )),
            line_assistant("Press Enter and I’ll ask the first question."),
        ],
        SetupScreen::AskProfile => vec![
            line_assistant(
                "I recommend a Personal Client profile to keep the first setup lightweight.",
            ),
            line_assistant("Keep that, or switch to a team or sandbox profile?"),
        ],
        SetupScreen::AskMemory => vec![
            line_assistant(
                "I recommend Filesystem memory so you can inspect everything locally first.",
            ),
            line_assistant("Keep that, or switch to a hosted-ready memory preset?"),
        ],
        SetupScreen::AskTools => vec![
            line_assistant("I’m going to keep these tools in sync from the same control plane."),
            line_assistant("Adjust the set only if you really want fewer managed tools."),
        ],
        SetupScreen::AskSkills => vec![
            line_assistant("You’re missing a shared skill layer right now."),
            line_assistant(
                "I suggest these starter skills so each tool follows the same working habits.",
            ),
        ],
        SetupScreen::AskMcps => vec![
            line_assistant(
                "I also want to install a minimal MCP layer so your tools can share consistent capabilities.",
            ),
            line_assistant("Keep my recommendation, or trim it down."),
        ],
        SetupScreen::Confirm => vec![
            line_assistant(
                "I’m ready to write the control plane, attach this project, seed memory if needed, and sync the managed outputs.",
            ),
            line_assistant("Press Enter if you want me to finish everything now."),
        ],
        SetupScreen::Complete => vec![line_assistant(
            "I finished the control plane setup. You can reopen the dashboard any time with `openagents-kit`.",
        )],
    }
}

fn option_title(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "Boot",
        SetupScreen::Detection => "Recommendation",
        SetupScreen::AskProfile => "Choose Profile",
        SetupScreen::AskMemory => "Choose Memory",
        SetupScreen::AskTools => "Managed Tools",
        SetupScreen::AskSkills => "Suggested Skills",
        SetupScreen::AskMcps => "Suggested MCP Servers",
        SetupScreen::Confirm => "Write Plan",
        SetupScreen::Complete => "What I Wrote",
    }
}

fn option_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![
            line_plain("I’m checking local tools, memory hints, and starter capabilities."),
            line_accent("Press Enter to skip ahead"),
        ],
        SetupScreen::Detection => vec![
            line_accent(format!(
                "Recommended profile   {}",
                app.selection.profile_preset.label()
            )),
            line_accent(format!(
                "Recommended memory    {}",
                app.selection.memory_backend.label()
            )),
            line_accent(format!(
                "Recommended tools     {}",
                join_or_none(
                    &app.selection
                        .enabled_tools
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                )
            )),
            line_plain(""),
            line_plain("Press Enter and I’ll walk you through the final decisions one by one."),
        ],
        SetupScreen::Confirm => confirm_lines(app),
        SetupScreen::Complete => complete_lines(app),
        _ => vec![],
    }
}

fn option_items(app: &SetupApp) -> Vec<ListItem<'static>> {
    match app.screen {
        SetupScreen::AskProfile => profile_items(app),
        SetupScreen::AskMemory => memory_items(app),
        SetupScreen::AskTools => tool_items(app),
        SetupScreen::AskSkills => catalog_items(
            skill_catalog(),
            &app.selection.selected_skills,
            app.skill_cursor,
        ),
        SetupScreen::AskMcps => catalog_items(
            mcp_catalog(),
            &app.selection.selected_mcp_servers,
            app.mcp_cursor,
        ),
        _ => vec![],
    }
}

fn profile_items(app: &SetupApp) -> Vec<ListItem<'static>> {
    [
        (
            ProfilePreset::PersonalClient,
            "Personal Client",
            "Good for one operator helping one client with a lightweight shared setup.",
        ),
        (
            ProfilePreset::TeamWorkspace,
            "Team Workspace",
            "Good when several people share the same capabilities and handoff flow.",
        ),
        (
            ProfilePreset::ProjectSandbox,
            "Project Sandbox",
            "Good when you want a temporary but still guided environment.",
        ),
    ]
    .into_iter()
    .map(|(preset, label, description)| {
        let prefix = if app.selection.profile_preset == preset {
            ">"
        } else {
            " "
        };
        ListItem::new(vec![
            line_with_prefix(prefix, label, app.selection.profile_preset == preset),
            line_subtle(description),
        ])
    })
    .collect()
}

fn memory_items(app: &SetupApp) -> Vec<ListItem<'static>> {
    [
        (
            MemoryBackendPreset::Filesystem,
            "Filesystem",
            "Stores memory inside the OpenAgents config directory so you can inspect it directly.",
        ),
        (
            MemoryBackendPreset::Cortex,
            "Cortex",
            "Keeps the control plane ready for a hosted or shared memory backend later.",
        ),
    ]
    .into_iter()
    .map(|(preset, label, description)| {
        let prefix = if app.selection.memory_backend == preset {
            ">"
        } else {
            " "
        };
        ListItem::new(vec![
            line_with_prefix(prefix, label, app.selection.memory_backend == preset),
            line_subtle(description),
        ])
    })
    .collect()
}

fn tool_items(app: &SetupApp) -> Vec<ListItem<'static>> {
    tool_order()
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let selected = app.selection.enabled_tools.contains(tool);
            let focused = app.tool_cursor == index;
            ListItem::new(vec![
                line_with_prefix(
                    if focused { ">" } else { " " },
                    &format!("[{}] {}", if selected { "x" } else { " " }, tool),
                    focused,
                ),
                line_subtle(tool_description(*tool)),
            ])
        })
        .collect()
}

fn catalog_items(
    items: Vec<&'static CatalogItem>,
    selected: &[String],
    cursor: usize,
) -> Vec<ListItem<'static>> {
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let enabled = selected.contains(&item.id.to_string());
            let focused = cursor == index;
            ListItem::new(vec![
                line_with_prefix(
                    if focused { ">" } else { " " },
                    &format!("[{}] {}", if enabled { "x" } else { " " }, item.name),
                    focused,
                ),
                line_subtle(item.description),
            ])
        })
        .collect()
}

fn confirm_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let config_path = runtime::resolve_config_path(None).unwrap_or_else(|_| "config.yaml".into());
    let managed_root = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| ".".into())
        .join("managed");

    let mut lines = vec![
        line_accent(format!("config.yaml   {}", config_path.display())),
        line_accent(format!("managed root  {}", managed_root.display())),
        line_plain(format!(
            "profile       {}",
            app.selection.profile_preset.profile_name()
        )),
        line_plain(format!(
            "memory        {}",
            app.selection.memory_backend.label()
        )),
        line_plain(format!(
            "tools         {}",
            join_or_none(
                &app.selection
                    .enabled_tools
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            )
        )),
        line_plain(format!(
            "skills        {}",
            join_or_none(&app.selection.selected_skills)
        )),
        line_plain(format!(
            "mcp servers   {}",
            join_or_none(&app.selection.selected_mcp_servers)
        )),
    ];
    for warning in &app.selection.warnings {
        lines.push(line_subtle(warning));
    }
    lines
}

fn complete_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let Some(summary) = &app.completion else {
        return vec![line_plain("Setup is complete.")];
    };

    let mut lines = vec![
        line_accent(format!("profile      {}", summary.profile_name)),
        line_accent(format!("managed root {}", summary.managed_root.display())),
        line_plain(format!(
            "tool files   {}",
            join_paths_or_none(&summary.tool_paths)
        )),
        line_plain(format!(
            "skill files  {}",
            join_paths_or_none(&summary.skill_paths)
        )),
        line_plain(format!(
            "mcp files    {}",
            join_paths_or_none(&summary.mcp_paths)
        )),
    ];
    if let Some(memory_path) = &summary.memory_path {
        lines.push(line_plain(format!(
            "memory       {}",
            memory_path.display()
        )));
    }
    lines.push(line_plain(""));
    lines.push(line_assistant(
        "Suggested next step: run `openagents-kit doctor` once to confirm the global profile is healthy on this device.",
    ));
    lines
}

pub fn screen_status(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "I am still scanning this device.",
        SetupScreen::Detection => "I am ready to start the interview.",
        SetupScreen::AskProfile => "Choose the profile you want me to manage.",
        SetupScreen::AskMemory => "Choose the memory layer you want me to provision.",
        SetupScreen::AskTools => "Choose which tools should stay in sync.",
        SetupScreen::AskSkills => "Choose the starter skills I should install.",
        SetupScreen::AskMcps => "Choose the starter MCP servers I should install.",
        SetupScreen::Confirm => "I’m ready to write and sync everything.",
        SetupScreen::Complete => "The control plane is ready.",
    }
}

pub fn setup_controls(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "Enter skip boot | q quit",
        SetupScreen::Detection => "Enter continue | q quit",
        SetupScreen::AskProfile | SetupScreen::AskMemory => {
            "Left/right switch | Enter continue | q quit"
        }
        SetupScreen::AskTools | SetupScreen::AskSkills | SetupScreen::AskMcps => {
            "Up/down move | Space toggle | Enter continue | Backspace back | q quit"
        }
        SetupScreen::Confirm => "Enter write control plane | Backspace back | q quit",
        SetupScreen::Complete => "Enter exit | q quit",
    }
}

pub fn boot_loading_message(tick: usize) -> String {
    let dots = ".".repeat((tick % 3) + 1);
    format!("Scanning local tools, memory hints, and starter capabilities{dots}")
}

fn tool_order() -> [ToolKind; 3] {
    [ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini]
}

fn skill_catalog() -> Vec<&'static CatalogItem> {
    curated_items()
        .iter()
        .filter(|item| item.kind == CatalogItemKind::Skill)
        .collect()
}

fn mcp_catalog() -> Vec<&'static CatalogItem> {
    curated_items()
        .iter()
        .filter(|item| item.kind == CatalogItemKind::Mcp)
        .collect()
}

fn tool_description(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Codex => "OpenAI Codex CLI managed from the same control plane.",
        ToolKind::Claude => "Anthropic Claude Code config and guidance kept in sync.",
        ToolKind::Gemini => "Gemini CLI starter outputs and managed capability inventory.",
    }
}

fn panel(title: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(SLATE))
        .style(Style::default().bg(CHARCOAL).fg(IVORY))
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    let backend = CrosstermBackend::new(stdout());
    Terminal::new(backend).context("failed to initialize terminal")
}

fn teardown_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    terminal.show_cursor().context("failed to restore cursor")?;
    Ok(())
}

fn line_plain<T: Into<String>>(text: T) -> Line<'static> {
    Line::from(Span::styled(text.into(), Style::default().fg(IVORY)))
}

fn line_accent<T: Into<String>>(text: T) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::default().fg(LIME).add_modifier(Modifier::BOLD),
    ))
}

fn line_assistant<T: Into<String>>(text: T) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "OpenAgents> ",
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.into(), Style::default().fg(IVORY)),
    ])
}

fn line_user<T: Into<String>>(text: T) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "You> ",
            Style::default().fg(LIME).add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.into(), Style::default().fg(IVORY)),
    ])
}

fn line_subtle<T: Into<String>>(text: T) -> Line<'static> {
    Line::from(Span::styled(text.into(), Style::default().fg(SLATE)))
}

fn line_with_prefix(prefix: &str, text: &str, highlighted: bool) -> Line<'static> {
    let style = if highlighted {
        Style::default().fg(LIME).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(IVORY)
    };
    Line::from(vec![
        Span::styled(format!("{prefix} "), Style::default().fg(TEAL)),
        Span::styled(text.to_string(), style),
    ])
}

fn join_paths_or_none(paths: &[std::path::PathBuf]) -> String {
    if paths.is_empty() {
        "none".to_string()
    } else {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn join_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BOOT_TICKS, SetupApp, SetupScreen, boot_loading_message, screen_status, setup_controls,
    };
    use crate::detection::DetectionReport;
    use crate::setup::{MemoryBackendPreset, ProfilePreset, SetupSelection};

    #[test]
    fn boot_message_animates() {
        assert!(boot_loading_message(0).ends_with('.'));
        assert!(boot_loading_message(1).ends_with(".."));
    }

    #[test]
    fn setup_status_and_controls_cover_new_skill_and_mcp_steps() {
        assert!(screen_status(SetupScreen::AskSkills).contains("starter skills"));
        assert!(screen_status(SetupScreen::AskMcps).contains("MCP servers"));
        assert!(setup_controls(SetupScreen::AskSkills).contains("Space toggle"));
    }

    #[test]
    fn setup_boot_advances_to_detection() {
        let selection = SetupSelection {
            workspace_name: "openagents-home".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![],
            selected_skills: vec![],
            selected_mcp_servers: vec![],
            warnings: vec![],
        };
        let mut app = SetupApp::new(DetectionReport::default(), selection);
        for _ in 0..BOOT_TICKS {
            app.advance_boot();
        }

        assert_eq!(app.screen, SetupScreen::Detection);
    }
}
