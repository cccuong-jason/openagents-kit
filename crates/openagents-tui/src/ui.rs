use std::io::stdout;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use openagents_core::ToolKind;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::catalog::{CatalogItem, curated_items};
use crate::control::ControlPlane;
use crate::detection::DetectionReport;
use crate::runtime::{self, SyncSummary};
use crate::setup::{
    MemoryBackendPreset, ProfilePreset, SetupQuestion, SetupSelection, setup_questions,
};

const BOOT_TICKS: u16 = 10;
const TEAL: Color = Color::Rgb(91, 214, 215);
const LIME: Color = Color::Rgb(194, 255, 121);
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

#[derive(Debug, Clone)]
struct AnsweredTurn {
    question: SetupQuestion,
    answer: String,
}

pub struct SetupApp {
    pub report: DetectionReport,
    pub selection: SetupSelection,
    pub existing_control_plane: bool,
    pub questions: Vec<SetupQuestion>,
    pub screen: SetupScreen,
    pub boot_tick: u16,
    pub motion_tick: usize,
    pub status: String,
    pub completion: Option<SyncSummary>,
    answered_turns: Vec<AnsweredTurn>,
    current_question_index: usize,
}

impl SetupApp {
    pub fn new(
        report: DetectionReport,
        selection: SetupSelection,
        existing_control_plane: bool,
    ) -> Self {
        Self {
            questions: setup_questions(&report, &selection, existing_control_plane),
            report,
            selection,
            existing_control_plane,
            screen: SetupScreen::Boot,
            boot_tick: 0,
            motion_tick: 0,
            status: boot_loading_message(0),
            completion: None,
            answered_turns: Vec::new(),
            current_question_index: 0,
        }
    }

    pub fn advance_boot(&mut self) {
        if self.screen != SetupScreen::Boot {
            return;
        }
        self.boot_tick = self.boot_tick.saturating_add(1);
        self.status = boot_loading_message(self.boot_tick.into());
        if self.boot_tick >= BOOT_TICKS {
            self.enter_screen(SetupScreen::Detection);
        }
    }

    pub fn tick_motion(&mut self) {
        if self.screen == SetupScreen::Boot {
            self.advance_boot();
            return;
        }
        self.motion_tick = self.motion_tick.saturating_add(1);
    }

    fn enter_screen(&mut self, screen: SetupScreen) {
        self.screen = screen;
        self.motion_tick = 0;
        self.status = screen_status(screen).to_string();
    }

    fn begin_questions(&mut self) {
        self.current_question_index = 0;
        if let Some(question) = self.questions.first().copied() {
            self.enter_screen(screen_for_question(question));
        } else {
            self.enter_screen(SetupScreen::Confirm);
        }
    }

    fn advance_from_question(&mut self, answer: impl Into<String>) {
        if let Some(question) = self.current_question() {
            self.answered_turns.push(AnsweredTurn {
                question,
                answer: answer.into(),
            });
        }

        self.current_question_index += 1;
        if let Some(question) = self.questions.get(self.current_question_index).copied() {
            self.enter_screen(screen_for_question(question));
        } else {
            self.enter_screen(SetupScreen::Confirm);
        }
    }

    pub fn previous_screen(&mut self) {
        if self.screen == SetupScreen::Detection || self.screen == SetupScreen::Boot {
            return;
        }
        if self.screen == SetupScreen::Confirm && self.current_question_index == 0 {
            self.enter_screen(SetupScreen::Detection);
            return;
        }

        if self.current_question_index == 0 {
            self.enter_screen(SetupScreen::Detection);
            return;
        }

        self.current_question_index -= 1;
        if !self.answered_turns.is_empty() {
            self.answered_turns.pop();
        }
        if let Some(question) = self.questions.get(self.current_question_index).copied() {
            self.enter_screen(screen_for_question(question));
        }
    }

    fn current_question(&self) -> Option<SetupQuestion> {
        self.questions.get(self.current_question_index).copied()
    }

    fn collapsed_turns(&self) -> usize {
        self.answered_turns.len()
    }

    fn apply_choice(&mut self, digit: usize) {
        match self.screen {
            SetupScreen::AskProfile => {
                if let Some(preset) = profile_preset_from_choice(digit) {
                    self.selection.profile_preset = preset;
                    crate::setup::refresh_catalog_recommendations(&mut self.selection);
                    self.advance_from_question(format!("Use profile: {}", preset.label()));
                }
            }
            SetupScreen::AskMemory => {
                if let Some(preset) = memory_preset_from_choice(digit) {
                    self.selection.memory_backend = preset;
                    crate::setup::refresh_catalog_recommendations(&mut self.selection);
                    self.advance_from_question(format!("Use memory: {}", preset.label()));
                }
            }
            SetupScreen::AskTools => {
                if let Some(tool) = tool_from_choice(digit) {
                    toggle_item(&mut self.selection.enabled_tools, tool);
                }
            }
            SetupScreen::AskSkills => {
                if let Some(item) = skill_catalog().get(digit.saturating_sub(1)) {
                    toggle_string(&mut self.selection.selected_skills, item.id);
                }
            }
            SetupScreen::AskMcps => {
                if let Some(item) = mcp_catalog().get(digit.saturating_sub(1)) {
                    toggle_string(&mut self.selection.selected_mcp_servers, item.id);
                }
            }
            SetupScreen::Detection => {
                if digit == 1 {
                    self.begin_questions();
                }
            }
            SetupScreen::Confirm => {
                if digit == 1 {
                    self.status = "Press Enter and I’ll write the control plane.".to_string();
                }
            }
            _ => {}
        }
    }

    fn confirm_current_selection(&mut self) {
        match self.screen {
            SetupScreen::Detection => self.begin_questions(),
            SetupScreen::AskProfile => {
                let preset = self.selection.profile_preset;
                self.advance_from_question(format!("Use profile: {}", preset.label()));
            }
            SetupScreen::AskMemory => {
                let preset = self.selection.memory_backend;
                self.advance_from_question(format!("Use memory: {}", preset.label()));
            }
            SetupScreen::AskTools => {
                self.advance_from_question(format!(
                    "Manage tools: {}",
                    join_or_none(
                        &self
                            .selection
                            .enabled_tools
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    )
                ));
            }
            SetupScreen::AskSkills => {
                self.advance_from_question(format!(
                    "Install skills: {}",
                    join_or_none(&self.selection.selected_skills)
                ));
            }
            SetupScreen::AskMcps => {
                self.advance_from_question(format!(
                    "Install MCP servers: {}",
                    join_or_none(&self.selection.selected_mcp_servers)
                ));
            }
            _ => {}
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
    let (report, selection, existing_control_plane) =
        runtime::recommended_setup_selection(config_override, manifest_override, cwd)?;

    if dry_run {
        let config = crate::setup::selection_to_config(&selection);
        println!("{}", serde_yaml::to_string(&config)?);
        return Ok(());
    }

    enable_raw_mode().context("failed to enable raw mode")?;
    let mut terminal = setup_terminal()?;
    let mut app = SetupApp::new(report, selection, existing_control_plane);
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

        if event::poll(Duration::from_millis(90))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Backspace | KeyCode::Char('b') => match app.screen {
                        SetupScreen::AskProfile
                        | SetupScreen::AskMemory
                        | SetupScreen::AskTools
                        | SetupScreen::AskSkills
                        | SetupScreen::AskMcps
                        | SetupScreen::Confirm => app.previous_screen(),
                        _ => {}
                    },
                    KeyCode::Enter => match app.screen {
                        SetupScreen::Boot => app.enter_screen(SetupScreen::Detection),
                        SetupScreen::Detection
                        | SetupScreen::AskProfile
                        | SetupScreen::AskMemory
                        | SetupScreen::AskTools
                        | SetupScreen::AskSkills
                        | SetupScreen::AskMcps => app.confirm_current_selection(),
                        SetupScreen::Confirm => {
                            match runtime::apply_setup(config_override, cwd, &app.selection) {
                                Ok(summary) => {
                                    app.completion = Some(summary);
                                    let transcript = setup_history(app);
                                    let _ =
                                        runtime::write_setup_history(config_override, &transcript);
                                    app.enter_screen(SetupScreen::Complete);
                                }
                                Err(error) => {
                                    app.status = format!("I could not finish setup: {error}");
                                }
                            }
                        }
                        SetupScreen::Complete => return Ok(()),
                    },
                    KeyCode::Char(value) if value.is_ascii_digit() => {
                        if let Some(digit) = value.to_digit(10) {
                            app.apply_choice(digit as usize);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            app.tick_motion();
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
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break Ok(());
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
        Paragraph::new(hero_lines(HeroState::Ready, "OpenAgents Control Center", 0))
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
            line_accent("openagents-kit history"),
            line_plain("Review the latest guided setup transcript after onboarding."),
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
            Constraint::Length(11),
            Constraint::Min(12),
            Constraint::Length(3),
        ])
        .split(area);

    let hero_state = match app.screen {
        SetupScreen::Boot => HeroState::Scanning,
        SetupScreen::Complete => HeroState::Ready,
        _ => HeroState::Listening,
    };

    frame.render_widget(
        Paragraph::new(hero_lines(
            hero_state,
            hero_title(app.screen, app.existing_control_plane),
            app.boot_tick as usize + app.motion_tick,
        ))
        .block(panel("OpenAgents"))
        .wrap(Wrap { trim: false }),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(setup_body_lines(app))
            .block(panel(body_title(app.screen)))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(setup_controls(app.screen), Style::default().fg(SLATE)),
            Span::raw("  "),
            Span::styled(&app.status, Style::default().fg(TEAL)),
        ]))
        .block(panel("Controls")),
        chunks[2],
    );
}

fn hero_title(screen: SetupScreen, existing_control_plane: bool) -> &'static str {
    match screen {
        SetupScreen::Boot => "OpenAgents is checking this device",
        SetupScreen::Detection if existing_control_plane => {
            "I found your existing OpenAgents control plane"
        }
        SetupScreen::Detection => "I found the starting point for your setup",
        SetupScreen::AskProfile => "Let me shape the profile first",
        SetupScreen::AskMemory => "Now I need a memory layer",
        SetupScreen::AskTools => "I only need to touch the tool gaps",
        SetupScreen::AskSkills => "I found a missing shared skill layer",
        SetupScreen::AskMcps => "I found missing MCP capabilities",
        SetupScreen::Confirm => "I’m ready to write the missing pieces",
        SetupScreen::Complete => "Your OpenAgents control plane is ready",
    }
}

fn body_title(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "Boot",
        SetupScreen::Detection => "Assistant",
        SetupScreen::AskProfile
        | SetupScreen::AskMemory
        | SetupScreen::AskTools
        | SetupScreen::AskSkills
        | SetupScreen::AskMcps => "Current Turn",
        SetupScreen::Confirm => "Ready To Apply",
        SetupScreen::Complete => "Completed",
    }
}

fn hero_lines(state: HeroState, title: &str, tick: usize) -> Vec<Line<'static>> {
    let pulse = match tick % 3 {
        0 => "·",
        1 => "•",
        _ => "●",
    };
    let visor = match state {
        HeroState::Scanning => "[]==[]",
        HeroState::Listening => "[====]",
        HeroState::Ready => "[^^^^]",
    };

    vec![
        line_accent(format!("OpenAgents // {title}")),
        line_subtle(
            "................................................................................",
        ),
        line_plain(""),
        line_plain("       .-----------------------------------------------------------------."),
        line_plain(format!(
            "       |  {pulse}  OPENAGENTS SETUP                                {pulse}  {pulse}  {pulse}       |"
        )),
        line_plain("       |                                                               .-. |"),
        line_plain(format!(
            "       |      .----.        {visor}                    .------.      (   )|"
        )),
        line_plain("       |     /|_||_|\\       .--.         .--------.     | sync |      `-` ||"),
        line_plain("       |     ||____||      /_||_\\        | memory |     | ctrl |    .---. ||"),
        line_plain(
            "       |_____|/____\\|______\\____/________| layer  |_____| plan |____|___|_||",
        ),
        line_subtle("                 One active turn at a time. Earlier turns collapse upward."),
    ]
}

fn setup_body_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if app.collapsed_turns() > 0 {
        lines.push(line_subtle(format!(
            "{} earlier turn(s) collapsed. Run `openagents-kit history` later to see the full setup transcript.",
            app.collapsed_turns()
        )));
        lines.push(line_plain(""));
    }

    let active = revealed_lines(active_turn_lines(app), app.motion_tick);
    lines.extend(active);

    let choices = choice_lines(app);
    if !choices.is_empty() {
        lines.push(line_plain(""));
        lines.extend(choices);
    }

    if matches!(app.screen, SetupScreen::Confirm | SetupScreen::Complete) {
        lines.push(line_plain(""));
        lines.extend(summary_lines(app));
    }

    lines
}

pub fn active_turn_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![
            line_assistant(
                "I’m scanning your local tools, memory hints, and shared capability clues.",
            ),
            line_assistant("Press Enter if you want me to skip the rest of the boot animation."),
        ],
        SetupScreen::Detection => detection_lines(app),
        SetupScreen::AskProfile => vec![
            line_assistant(
                "I recommend a Personal Client profile to keep the first setup lightweight.",
            ),
            line_assistant(
                "If you want a different default, pick it below. Otherwise press Enter and I’ll keep my recommendation.",
            ),
        ],
        SetupScreen::AskMemory => vec![
            line_assistant(
                "I recommend Filesystem memory so you can inspect everything locally first.",
            ),
            line_assistant("Keep that, or switch to a hosted-ready memory preset."),
        ],
        SetupScreen::AskTools => vec![
            line_assistant(
                "I only need to manage the tools that are missing or uncertain right now.",
            ),
            line_assistant(
                "Toggle any tool you do not want me to manage, then press Enter when this list looks right.",
            ),
        ],
        SetupScreen::AskSkills => vec![
            line_assistant(
                "I found missing shared skills, so I’m proposing a small starter layer.",
            ),
            line_assistant("Toggle anything you do not want, then press Enter to continue."),
        ],
        SetupScreen::AskMcps => vec![
            line_assistant(
                "I found missing MCP capabilities, so I’m proposing only the pieces this setup still needs.",
            ),
            line_assistant("Toggle anything you do not want, then press Enter to continue."),
        ],
        SetupScreen::Confirm => vec![
            line_assistant(
                "I’m ready to write the control plane, attach this project, seed memory if needed, and sync the managed outputs.",
            ),
            line_assistant(
                "Press Enter to finish. If you want to revisit the last question, press Backspace.",
            ),
        ],
        SetupScreen::Complete => vec![
            line_assistant(
                "I finished the missing setup work and recorded this session for later review.",
            ),
            line_assistant(
                "You can reopen the dashboard with `openagents-kit`, or inspect the transcript with `openagents-kit history`.",
            ),
        ],
    }
}

fn detection_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let mut lines = vec![line_assistant(
        "Welcome. I’m going to keep one OpenAgents control plane aligned across your tools, skills, MCP servers, and memory.",
    )];

    if app.existing_control_plane {
        lines.push(line_assistant(
            "I found an existing control plane, so I’m only going to ask about the gaps that still need attention.",
        ));
    } else {
        lines.push(line_assistant(
            "I do not see a saved OpenAgents control plane yet, so I prepared a starter setup based on what I detected.",
        ));
    }

    if app.report.detections.is_empty() {
        lines.push(line_assistant(
            "I did not find a strong existing tool footprint, so I prepared a safe cross-tool starter.",
        ));
    } else {
        lines.push(line_assistant(format!(
            "I found {} supported tool(s): {}.",
            app.report.detections.len(),
            app.report
                .detections
                .iter()
                .map(|item| item.tool.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    if !app.report.has_memory_layer {
        lines.push(line_assistant(
            "You do not have a consistent memory layer yet, so I’m going to fix that first.",
        ));
    }

    if app.questions == vec![SetupQuestion::Confirm] {
        lines.push(line_assistant(
            "Everything important is already configured, so I only need your confirmation before I resync the managed outputs.",
        ));
    } else {
        lines.push(line_assistant(
            "Press Enter and I’ll walk through only the missing pieces.",
        ));
    }

    lines
}

fn choice_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![line_subtle("Enter skip boot animation  |  q quit")],
        SetupScreen::Detection => vec![
            line_choice(1, "Start the guided setup"),
            line_subtle("Enter also accepts this default."),
        ],
        SetupScreen::AskProfile => profile_choice_lines(app),
        SetupScreen::AskMemory => memory_choice_lines(app),
        SetupScreen::AskTools => toggle_choice_lines(
            &app.selection
                .enabled_tools
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            tool_order()
                .iter()
                .map(|tool| (tool.to_string(), app.selection.enabled_tools.contains(tool)))
                .collect(),
        ),
        SetupScreen::AskSkills => toggle_choice_lines(
            &app.selection.selected_skills,
            skill_catalog()
                .into_iter()
                .map(|item| {
                    (
                        format!("{} — {}", item.name, item.description),
                        app.selection
                            .selected_skills
                            .iter()
                            .any(|value| value == item.id),
                    )
                })
                .collect(),
        ),
        SetupScreen::AskMcps => toggle_choice_lines(
            &app.selection.selected_mcp_servers,
            mcp_catalog()
                .into_iter()
                .map(|item| {
                    (
                        format!("{} — {}", item.name, item.description),
                        app.selection
                            .selected_mcp_servers
                            .iter()
                            .any(|value| value == item.id),
                    )
                })
                .collect(),
        ),
        SetupScreen::Confirm => vec![
            line_choice(1, "Write the control plane and sync the managed outputs"),
            line_subtle("Enter accepts this plan. Backspace returns to the previous question."),
        ],
        SetupScreen::Complete => vec![line_subtle("Enter exit  |  q quit")],
    }
}

fn profile_choice_lines(app: &SetupApp) -> Vec<Line<'static>> {
    [
        (
            ProfilePreset::PersonalClient,
            "Personal Client",
            "One operator, one client, low ceremony.",
        ),
        (
            ProfilePreset::TeamWorkspace,
            "Team Workspace",
            "Shared capabilities and handoff-friendly defaults.",
        ),
        (
            ProfilePreset::ProjectSandbox,
            "Project Sandbox",
            "Temporary but still guided and synchronized.",
        ),
    ]
    .into_iter()
    .enumerate()
    .flat_map(|(index, (preset, label, description))| {
        let selected = app.selection.profile_preset == preset;
        vec![
            line_choice_state(index + 1, label, selected),
            line_subtle(description),
        ]
    })
    .chain([line_subtle(
        "Enter keeps the currently selected recommendation.",
    )])
    .collect()
}

fn memory_choice_lines(app: &SetupApp) -> Vec<Line<'static>> {
    [
        (
            MemoryBackendPreset::Filesystem,
            "Filesystem",
            "Stored inside the OpenAgents config root so you can inspect it directly.",
        ),
        (
            MemoryBackendPreset::Cortex,
            "Cortex",
            "Hosted-ready memory preset for later expansion.",
        ),
    ]
    .into_iter()
    .enumerate()
    .flat_map(|(index, (preset, label, description))| {
        let selected = app.selection.memory_backend == preset;
        vec![
            line_choice_state(index + 1, label, selected),
            line_subtle(description),
        ]
    })
    .chain([line_subtle(
        "Enter keeps the currently selected recommendation.",
    )])
    .collect()
}

fn toggle_choice_lines(selected_ids: &[String], rows: Vec<(String, bool)>) -> Vec<Line<'static>> {
    let mut lines = rows
        .into_iter()
        .enumerate()
        .map(|(index, (label, selected))| {
            line_choice_state(
                index + 1,
                &format!("[{}] {}", if selected { "x" } else { " " }, label),
                selected,
            )
        })
        .collect::<Vec<_>>();
    lines.push(line_subtle(format!(
        "Enter continues with: {}",
        join_or_none(selected_ids)
    )));
    lines
}

fn summary_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Confirm => {
            let config_path =
                runtime::resolve_config_path(None).unwrap_or_else(|_| "config.yaml".into());
            let managed_root = config_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| ".".into())
                .join("managed");

            let mut lines = vec![
                line_accent(format!("config      {}", config_path.display())),
                line_accent(format!("managed     {}", managed_root.display())),
                line_plain(format!(
                    "profile     {}",
                    app.selection.profile_preset.profile_name()
                )),
                line_plain(format!(
                    "memory      {}",
                    app.selection.memory_backend.label()
                )),
                line_plain(format!(
                    "tools       {}",
                    join_or_none(
                        &app.selection
                            .enabled_tools
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    )
                )),
                line_plain(format!(
                    "skills      {}",
                    join_or_none(&app.selection.selected_skills)
                )),
                line_plain(format!(
                    "mcp         {}",
                    join_or_none(&app.selection.selected_mcp_servers)
                )),
            ];
            for warning in &app.selection.warnings {
                lines.push(line_subtle(warning));
            }
            lines
        }
        SetupScreen::Complete => {
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
            lines
        }
        _ => Vec::new(),
    }
}

fn screen_for_question(question: SetupQuestion) -> SetupScreen {
    match question {
        SetupQuestion::Profile => SetupScreen::AskProfile,
        SetupQuestion::Memory => SetupScreen::AskMemory,
        SetupQuestion::Tools => SetupScreen::AskTools,
        SetupQuestion::Skills => SetupScreen::AskSkills,
        SetupQuestion::Mcps => SetupScreen::AskMcps,
        SetupQuestion::Confirm => SetupScreen::Confirm,
    }
}

pub fn screen_status(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "I am still scanning this device.",
        SetupScreen::Detection => "I am ready to start the guided setup.",
        SetupScreen::AskProfile => "Pick the profile you want me to use.",
        SetupScreen::AskMemory => "Pick the memory layer you want me to provision.",
        SetupScreen::AskTools => "Choose which tools I should manage.",
        SetupScreen::AskSkills => "Choose the shared skills I should install.",
        SetupScreen::AskMcps => "Choose the MCP servers I should install.",
        SetupScreen::Confirm => "I’m ready to write and sync everything.",
        SetupScreen::Complete => "The control plane is ready.",
    }
}

pub fn setup_controls(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "Enter skip boot | q quit",
        SetupScreen::Detection => "1 start | Enter continue | q quit",
        SetupScreen::AskProfile | SetupScreen::AskMemory => {
            "1-3 choose | Enter keep recommendation | Backspace back | q quit"
        }
        SetupScreen::AskTools | SetupScreen::AskSkills | SetupScreen::AskMcps => {
            "1-3 toggle | Enter continue | Backspace back | q quit"
        }
        SetupScreen::Confirm => "1 write | Enter confirm | Backspace back | q quit",
        SetupScreen::Complete => "Enter exit | q quit",
    }
}

pub fn boot_loading_message(tick: usize) -> String {
    let dots = ".".repeat((tick % 3) + 1);
    format!("Scanning local tools, memory hints, and starter capabilities{dots}")
}

fn profile_preset_from_choice(digit: usize) -> Option<ProfilePreset> {
    match digit {
        1 => Some(ProfilePreset::PersonalClient),
        2 => Some(ProfilePreset::TeamWorkspace),
        3 => Some(ProfilePreset::ProjectSandbox),
        _ => None,
    }
}

fn memory_preset_from_choice(digit: usize) -> Option<MemoryBackendPreset> {
    match digit {
        1 => Some(MemoryBackendPreset::Filesystem),
        2 => Some(MemoryBackendPreset::Cortex),
        _ => None,
    }
}

fn tool_from_choice(digit: usize) -> Option<ToolKind> {
    match digit {
        1 => Some(ToolKind::Codex),
        2 => Some(ToolKind::Claude),
        3 => Some(ToolKind::Gemini),
        _ => None,
    }
}

fn tool_order() -> [ToolKind; 3] {
    [ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini]
}

fn skill_catalog() -> Vec<&'static CatalogItem> {
    curated_items()
        .iter()
        .filter(|item| item.kind == openagents_core::CatalogItemKind::Skill)
        .collect()
}

fn mcp_catalog() -> Vec<&'static CatalogItem> {
    curated_items()
        .iter()
        .filter(|item| item.kind == openagents_core::CatalogItemKind::Mcp)
        .collect()
}

fn toggle_item<T: Ord + Clone + PartialEq>(items: &mut Vec<T>, item: T) {
    if let Some(index) = items.iter().position(|value| value == &item) {
        items.remove(index);
    } else {
        items.push(item);
        items.sort();
    }
}

fn toggle_string(items: &mut Vec<String>, item: &str) {
    toggle_item(items, item.to_string());
}

fn revealed_lines(lines: Vec<Line<'static>>, motion_tick: usize) -> Vec<Line<'static>> {
    let visible = if lines.is_empty() {
        0
    } else {
        usize::min(lines.len(), usize::max(1, (motion_tick / 2) + 1))
    };
    let total = lines.len();
    let mut revealed = lines.into_iter().take(visible).collect::<Vec<_>>();
    if visible < total {
        revealed.push(line_subtle("█"));
    }
    revealed
}

fn setup_history(app: &SetupApp) -> String {
    let mut lines = vec![
        "OpenAgents> Welcome. I’m going to keep one OpenAgents control plane aligned across your tools, skills, MCP servers, and memory.".to_string(),
    ];

    if app.existing_control_plane {
        lines.push("OpenAgents> I found an existing control plane, so I only asked about the gaps that still needed attention.".to_string());
    } else {
        lines.push("OpenAgents> I did not find a saved OpenAgents control plane yet, so I prepared a starter setup.".to_string());
    }

    if app.report.detections.is_empty() {
        lines.push("OpenAgents> I did not find a strong existing tool footprint.".to_string());
    } else {
        for detection in &app.report.detections {
            lines.push(format!(
                "OpenAgents> I found {} from {}.",
                detection.summary,
                detection.evidence_path.display()
            ));
        }
    }

    if !app.report.has_memory_layer {
        lines.push("OpenAgents> You did not have a consistent memory layer yet.".to_string());
    }

    for answered in &app.answered_turns {
        for line in question_history_lines(answered.question, app) {
            lines.push(line);
        }
        lines.push(format!("You> {}", answered.answer));
    }

    if app.completion.is_some() {
        lines.push(
            "OpenAgents> I finished the missing setup work and synchronized the managed outputs."
                .to_string(),
        );
    }

    lines.join("\n")
}

fn question_history_lines(question: SetupQuestion, app: &SetupApp) -> Vec<String> {
    match question {
        SetupQuestion::Profile => vec![
            "OpenAgents> I recommend a Personal Client profile to keep the first setup lightweight.".to_string(),
            "OpenAgents> If you want a different default, pick it below. Otherwise press Enter and I’ll keep my recommendation.".to_string(),
        ],
        SetupQuestion::Memory => vec![
            "OpenAgents> I recommend Filesystem memory so you can inspect everything locally first.".to_string(),
            "OpenAgents> Keep that, or switch to a hosted-ready memory preset.".to_string(),
        ],
        SetupQuestion::Tools => vec![
            "OpenAgents> I only need to manage the tools that are missing or uncertain right now.".to_string(),
            "OpenAgents> Toggle any tool you do not want me to manage, then press Enter when this list looks right.".to_string(),
        ],
        SetupQuestion::Skills => vec![
            "OpenAgents> I found missing shared skills, so I’m proposing a small starter layer.".to_string(),
            "OpenAgents> Toggle anything you do not want, then press Enter to continue.".to_string(),
        ],
        SetupQuestion::Mcps => vec![
            "OpenAgents> I found missing MCP capabilities, so I’m proposing only the pieces this setup still needs.".to_string(),
            "OpenAgents> Toggle anything you do not want, then press Enter to continue.".to_string(),
        ],
        SetupQuestion::Confirm => vec![format!(
            "OpenAgents> I’m ready to write the control plane for `{}`.",
            app.selection.workspace_name
        )],
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

fn line_subtle<T: Into<String>>(text: T) -> Line<'static> {
    Line::from(Span::styled(text.into(), Style::default().fg(SLATE)))
}

fn line_choice(number: usize, text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{number}. "),
            Style::default().fg(LIME).add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), Style::default().fg(IVORY)),
    ])
}

fn line_choice_state(number: usize, text: &str, selected: bool) -> Line<'static> {
    let style = if selected {
        Style::default().fg(LIME).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(IVORY)
    };
    Line::from(vec![
        Span::styled(
            format!("{number}. "),
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        ),
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
        BOOT_TICKS, HeroState, SetupApp, SetupScreen, active_turn_lines, boot_loading_message,
        hero_lines, revealed_lines, screen_status, setup_controls,
    };
    use crate::detection::DetectionReport;
    use crate::setup::{
        MemoryBackendPreset, ProfilePreset, SetupQuestion, SetupSelection, setup_questions,
    };
    use openagents_core::ToolKind;

    #[test]
    fn boot_message_animates() {
        assert!(boot_loading_message(0).ends_with('.'));
        assert!(boot_loading_message(1).ends_with(".."));
    }

    #[test]
    fn setup_status_and_controls_cover_new_skill_and_mcp_steps() {
        assert!(screen_status(SetupScreen::AskSkills).contains("shared skills"));
        assert!(screen_status(SetupScreen::AskMcps).contains("MCP"));
        assert!(setup_controls(SetupScreen::AskSkills).contains("1-3 toggle"));
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
        let mut app = SetupApp::new(DetectionReport::default(), selection, false);
        for _ in 0..BOOT_TICKS {
            app.advance_boot();
        }

        assert_eq!(app.screen, SetupScreen::Detection);
    }

    #[test]
    fn controls_use_numbered_choices_instead_of_arrow_navigation() {
        assert!(setup_controls(SetupScreen::AskProfile).contains("1-3 choose"));
        assert!(setup_controls(SetupScreen::AskTools).contains("1-3 toggle"));
        assert!(!setup_controls(SetupScreen::AskTools).contains("Space toggle"));
    }

    #[test]
    fn active_turn_does_not_repeat_previous_questions_inline() {
        let selection = SetupSelection {
            workspace_name: "openagents-home".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Codex],
            selected_skills: vec![],
            selected_mcp_servers: vec![],
            warnings: vec![],
        };
        let mut app = SetupApp::new(DetectionReport::default(), selection, false);
        app.questions = vec![
            SetupQuestion::Profile,
            SetupQuestion::Memory,
            SetupQuestion::Confirm,
        ];
        app.screen = SetupScreen::AskMemory;

        let lines = active_turn_lines(&app)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(lines.contains("Filesystem memory"));
        assert!(!lines.contains("Personal Client"));
    }

    #[test]
    fn app_uses_gap_driven_question_order() {
        let selection = SetupSelection {
            workspace_name: "openagents-home".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Codex],
            selected_skills: vec!["shared-memory".to_string()],
            selected_mcp_servers: vec!["filesystem-memory".to_string()],
            warnings: vec![],
        };
        let report = DetectionReport {
            detections: vec![],
            warnings: vec![],
            installed_skills: vec![],
            installed_mcp_servers: vec![],
            has_memory_layer: false,
        };

        assert_eq!(
            setup_questions(&report, &selection, false),
            vec![
                SetupQuestion::Profile,
                SetupQuestion::Memory,
                SetupQuestion::Tools,
                SetupQuestion::Skills,
                SetupQuestion::Mcps,
                SetupQuestion::Confirm
            ]
        );
    }

    #[test]
    fn reveal_animation_keeps_a_cursor_until_the_turn_is_fully_visible() {
        let partial = revealed_lines(hero_lines(HeroState::Listening, "Test", 0), 0);

        assert!(
            partial
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                .contains("█")
        );
    }
}
