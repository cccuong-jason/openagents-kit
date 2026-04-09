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
use ratatui::widgets::{Paragraph, Wrap};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HomeAction {
    Sync,
    Doctor,
    History,
    Setup,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HomeHeroState {
    Ready,
    Working,
}

#[derive(Debug, Clone)]
struct HomeApp {
    action_index: usize,
    status: String,
    intro: Option<String>,
    result_lines: Vec<Line<'static>>,
    hero_state: HomeHeroState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HomeOutcome {
    Exit,
    LaunchSetup,
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
    profile_cursor: usize,
    memory_cursor: usize,
    tool_cursor: usize,
    skill_cursor: usize,
    mcp_cursor: usize,
}

impl SetupApp {
    pub fn new(
        report: DetectionReport,
        selection: SetupSelection,
        existing_control_plane: bool,
    ) -> Self {
        let profile_cursor = profile_index(selection.profile_preset);
        let memory_cursor = memory_index(selection.memory_backend);
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
            profile_cursor,
            memory_cursor,
            tool_cursor: 0,
            skill_cursor: 0,
            mcp_cursor: 0,
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

    fn cycle_profile(&mut self, forward: bool) {
        self.profile_cursor = wrap_cursor(self.profile_cursor, 3, forward);
        self.selection.profile_preset = profile_preset_from_cursor(self.profile_cursor);
        crate::setup::refresh_catalog_recommendations(&mut self.selection);
    }

    fn cycle_memory(&mut self, forward: bool) {
        self.memory_cursor = wrap_cursor(self.memory_cursor, 2, forward);
        self.selection.memory_backend = memory_preset_from_cursor(self.memory_cursor);
        crate::setup::refresh_catalog_recommendations(&mut self.selection);
    }

    fn move_tool_cursor(&mut self, forward: bool) {
        self.tool_cursor = wrap_cursor(self.tool_cursor, tool_order().len(), forward);
    }

    fn move_skill_cursor(&mut self, forward: bool) {
        self.skill_cursor = wrap_cursor(self.skill_cursor, skill_catalog().len(), forward);
    }

    fn move_mcp_cursor(&mut self, forward: bool) {
        self.mcp_cursor = wrap_cursor(self.mcp_cursor, mcp_catalog().len(), forward);
    }

    fn toggle_current_tool(&mut self) {
        if let Some(tool) = tool_order().get(self.tool_cursor).copied() {
            toggle_item(&mut self.selection.enabled_tools, tool);
        }
    }

    fn toggle_current_skill(&mut self) {
        if let Some(item) = skill_catalog().get(self.skill_cursor) {
            toggle_string(&mut self.selection.selected_skills, item.id);
        }
    }

    fn toggle_current_mcp(&mut self) {
        if let Some(item) = mcp_catalog().get(self.mcp_cursor) {
            toggle_string(&mut self.selection.selected_mcp_servers, item.id);
        }
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
                    self.status = "Press Enter and I’ll write the setup changes.".to_string();
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
        Ok(control) => run_home(&control, cwd, None, config_override, manifest_override),
        Err(_) => run_setup(config_override, manifest_override, cwd, false),
    }
}

impl HomeApp {
    fn new(intro: Option<String>) -> Self {
        Self {
            action_index: 0,
            status: "Your setup is healthy and ready for the next step.".to_string(),
            intro,
            result_lines: Vec::new(),
            hero_state: HomeHeroState::Ready,
        }
    }

    fn move_action(&mut self, forward: bool) {
        self.action_index = wrap_cursor(self.action_index, home_actions().len(), forward);
    }

    fn current_action(&self) -> HomeAction {
        home_actions()[self.action_index]
    }

    fn show_result(
        &mut self,
        hero_state: HomeHeroState,
        status: String,
        result_lines: Vec<Line<'static>>,
    ) {
        self.hero_state = hero_state;
        self.status = status;
        self.result_lines = result_lines;
    }

    fn show_error(&mut self, status: String) {
        self.hero_state = HomeHeroState::Ready;
        self.status = status;
        self.result_lines.clear();
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

    if existing_control_plane && setup_questions(&report, &selection, true).is_empty() {
        let control = ControlPlane::load(config_override, manifest_override)?;
        return run_home(
            &control,
            cwd,
            Some(
                "I checked your saved setup and everything important is already aligned."
                    .to_string(),
            ),
            config_override,
            manifest_override,
        );
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
                    KeyCode::Left => match app.screen {
                        SetupScreen::AskProfile => app.cycle_profile(false),
                        SetupScreen::AskMemory => app.cycle_memory(false),
                        SetupScreen::AskTools => app.toggle_current_tool(),
                        SetupScreen::AskSkills => app.toggle_current_skill(),
                        SetupScreen::AskMcps => app.toggle_current_mcp(),
                        _ => {}
                    },
                    KeyCode::Right => match app.screen {
                        SetupScreen::AskProfile => app.cycle_profile(true),
                        SetupScreen::AskMemory => app.cycle_memory(true),
                        SetupScreen::AskTools => app.toggle_current_tool(),
                        SetupScreen::AskSkills => app.toggle_current_skill(),
                        SetupScreen::AskMcps => app.toggle_current_mcp(),
                        _ => {}
                    },
                    KeyCode::Up => match app.screen {
                        SetupScreen::AskTools => app.move_tool_cursor(false),
                        SetupScreen::AskSkills => app.move_skill_cursor(false),
                        SetupScreen::AskMcps => app.move_mcp_cursor(false),
                        _ => {}
                    },
                    KeyCode::Down => match app.screen {
                        SetupScreen::AskTools => app.move_tool_cursor(true),
                        SetupScreen::AskSkills => app.move_skill_cursor(true),
                        SetupScreen::AskMcps => app.move_mcp_cursor(true),
                        _ => {}
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

fn run_home(
    control: &ControlPlane,
    cwd: &Path,
    intro: Option<String>,
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
) -> Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut terminal = setup_terminal()?;
    let active_profile = control.active_profile_name(cwd, None);
    let resolved = control.resolved_profile(&active_profile)?;
    let report = runtime::load_detection_report()?;
    let mut app = HomeApp::new(intro);

    let result: Result<HomeOutcome> = loop {
        terminal.draw(|frame| draw_home(frame, &app, control, cwd, &resolved, &report))?;
        if event::poll(Duration::from_millis(110))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(HomeOutcome::Exit),
                KeyCode::Up => app.move_action(false),
                KeyCode::Down => app.move_action(true),
                KeyCode::Enter => match app.current_action() {
                    HomeAction::Sync => {
                        match runtime::sync_control_plane(control, &resolved.name, false) {
                            Ok(summary) => app.show_result(
                                HomeHeroState::Working,
                                "I resynced the managed outputs from your saved OpenAgents setup."
                                    .to_string(),
                                sync_result_lines(&summary),
                            ),
                            Err(error) => {
                                app.show_error(format!("I could not sync right now: {error}"))
                            }
                        }
                    }
                    HomeAction::Doctor => app.show_result(
                        HomeHeroState::Ready,
                        "I checked the current health of your saved setup.".to_string(),
                        doctor_result_lines(control, cwd, &resolved, &report),
                    ),
                    HomeAction::History => match runtime::read_setup_history(Some(&control.root)) {
                        Ok(history) => app.show_result(
                            HomeHeroState::Ready,
                            "Here is the latest stored setup transcript.".to_string(),
                            history_result_lines(&history),
                        ),
                        Err(error) => {
                            app.show_error(format!("I could not load setup history: {error}"))
                        }
                    },
                    HomeAction::Setup => break Ok(HomeOutcome::LaunchSetup),
                    HomeAction::Exit => break Ok(HomeOutcome::Exit),
                },
                _ => {}
            }
        }
    };
    teardown_terminal(&mut terminal)?;
    match result? {
        HomeOutcome::Exit => Ok(()),
        HomeOutcome::LaunchSetup => run_setup(config_override, manifest_override, cwd, false),
    }
}

fn draw_home(
    frame: &mut ratatui::Frame<'_>,
    app: &HomeApp,
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
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(hero_lines(
            match app.hero_state {
                HomeHeroState::Ready => HeroState::Ready,
                HomeHeroState::Working => HeroState::Scanning,
            },
            "Your setup is healthy and ready",
            app.action_index,
        ))
        .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let mut lines = home_intro_lines(app, control, cwd, profile, report);
    lines.push(line_plain(""));
    lines.extend(home_action_lines(app));
    if !app.result_lines.is_empty() {
        lines.push(line_plain(""));
        lines.extend(app.result_lines.clone());
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chunks[1]);
    frame.render_widget(
        Paragraph::new(line_subtle("↑/↓ choose  |  Enter run  |  q quit")),
        chunks[2],
    );
}

fn draw_setup(frame: &mut ratatui::Frame<'_>, app: &SetupApp) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Length(1),
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
        .wrap(Wrap { trim: false }),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(setup_body_lines(app)).wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(line_subtle(format!(
            "{}  |  {}",
            setup_controls(app.screen),
            app.status
        ))),
        chunks[2],
    );
}

fn hero_title(screen: SetupScreen, existing_control_plane: bool) -> &'static str {
    match screen {
        SetupScreen::Boot => "I’m scanning this device",
        SetupScreen::Detection if existing_control_plane => "I found your existing setup",
        SetupScreen::Detection => "I found the starting point for your setup",
        SetupScreen::AskProfile => "I’m shaping the profile",
        SetupScreen::AskMemory => "I’m filling the memory gap",
        SetupScreen::AskTools => "I’m lining up your tools",
        SetupScreen::AskSkills => "I found missing shared skills",
        SetupScreen::AskMcps => "I found missing MCP capabilities",
        SetupScreen::Confirm => "I’m ready to write the missing pieces",
        SetupScreen::Complete => "Your OpenAgents setup is ready",
    }
}

fn hero_lines(state: HeroState, title: &str, tick: usize) -> Vec<Line<'static>> {
    let stars = match tick % 3 {
        0 => "✦     ✦",
        1 => "  ✦ ✦  ",
        _ => "✦   ✦  ",
    };
    let moon = match state {
        HeroState::Scanning => "◐",
        HeroState::Listening => "◑",
        HeroState::Ready => "◉",
    };
    let eye_band = match state {
        HeroState::Scanning => "▀▀",
        HeroState::Listening => "██",
        HeroState::Ready => "▌▐",
    };

    vec![
        line_accent(format!("OpenAgents // {title}")),
        line_subtle(
            "────────────────────────────────────────────────────────────────────────────────",
        ),
        Line::from(vec![
            Span::styled(format!("   {stars:<12}"), Style::default().fg(SLATE)),
            Span::styled("░░░░░░░░░░", Style::default().fg(Color::Rgb(52, 72, 96))),
            Span::styled("                           ", Style::default().fg(IVORY)),
            Span::styled(moon.to_string(), Style::default().fg(LIME)),
        ]),
        Line::from(vec![
            Span::styled("      ▄▄▄▄▄▄▄▄▄      ", Style::default().fg(TEAL)),
            Span::styled(
                "           ░░░░░░░░          ",
                Style::default().fg(Color::Rgb(44, 58, 78)),
            ),
            Span::styled("✦", Style::default().fg(SLATE)),
        ]),
        Line::from(vec![
            Span::styled(
                "    ▄███████████▄    ",
                Style::default().fg(Color::Rgb(214, 149, 154)),
            ),
            Span::styled(
                "      ░░░░░░░░░       ",
                Style::default().fg(Color::Rgb(36, 49, 65)),
            ),
        ]),
        Line::from(vec![
            Span::styled("    ██", Style::default().fg(Color::Rgb(214, 149, 154))),
            Span::styled(
                format!("{eye_band:^4}"),
                Style::default().fg(CHARCOAL).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "███████    ",
                Style::default().fg(Color::Rgb(214, 149, 154)),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  ▄████████████████▄  ",
                Style::default().fg(Color::Rgb(214, 149, 154)),
            ),
            Span::styled(
                "   sync ctrl memory",
                Style::default().fg(if state == HeroState::Scanning {
                    LIME
                } else {
                    SLATE
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  ██████████████████  ",
                Style::default().fg(Color::Rgb(214, 149, 154)),
            ),
            Span::styled("  pixel setup scene", Style::default().fg(SLATE)),
        ]),
        Line::from(vec![
            Span::styled(
                "     ██   ██   ██     ",
                Style::default().fg(Color::Rgb(214, 149, 154)),
            ),
            Span::styled("                         ", Style::default().fg(IVORY)),
        ]),
        line_subtle(
            "────────────────────────────────────────────────────────────────────────────────",
        ),
    ]
}

fn setup_body_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let mut lines = revealed_lines(active_turn_lines(app), app.motion_tick);
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
                "Use the arrow keys to adjust the list, then press Enter when it looks right.",
            ),
        ],
        SetupScreen::AskSkills => vec![
            line_assistant(
                "I found missing shared skills, so I’m proposing a small starter layer.",
            ),
            line_assistant("Use the arrow keys to adjust the list, then press Enter to continue."),
        ],
        SetupScreen::AskMcps => vec![
            line_assistant(
                "I found missing MCP capabilities, so I’m proposing only the pieces this setup still needs.",
            ),
            line_assistant("Use the arrow keys to adjust the list, then press Enter to continue."),
        ],
        SetupScreen::Confirm => vec![
            line_assistant(
                "I’m ready to write the missing setup pieces, attach this project, seed memory if needed, and sync the managed outputs.",
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
                "You can reopen home with `openagents-kit`, or inspect the transcript with `openagents-kit history`.",
            ),
        ],
    }
}

fn detection_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let mut lines = vec![line_assistant(
        "Welcome. I’m going to keep one OpenAgents setup aligned across your tools, skills, MCP servers, and memory.",
    )];

    if app.existing_control_plane {
        lines.push(line_assistant(
            "I found your existing setup, so I’m only going to touch the pieces that are still missing or drifting.",
        ));
    } else {
        lines.push(line_assistant(
            "I do not see a saved OpenAgents setup yet, so I prepared a starter setup based on what I detected.",
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

    lines.push(line_assistant(
        "Press Enter and I’ll walk through only the missing pieces.",
    ));

    lines
}

fn choice_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![line_subtle("Enter skip boot animation  |  q quit")],
        SetupScreen::Detection => vec![line_subtle("Enter start guided setup")],
        SetupScreen::AskProfile => profile_choice_lines(app),
        SetupScreen::AskMemory => memory_choice_lines(app),
        SetupScreen::AskTools => toggle_choice_lines(
            app.tool_cursor,
            tool_order()
                .iter()
                .map(|tool| (tool.to_string(), app.selection.enabled_tools.contains(tool)))
                .collect(),
        ),
        SetupScreen::AskSkills => toggle_choice_lines(
            app.skill_cursor,
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
            app.mcp_cursor,
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
        SetupScreen::Confirm => vec![line_subtle(
            "Enter write the missing setup pieces. Backspace returns to the previous question.",
        )],
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
        let focused = app.profile_cursor == index;
        let selected = app.selection.profile_preset == preset;
        vec![
            line_selectable_option(label, focused, selected),
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
        let focused = app.memory_cursor == index;
        let selected = app.selection.memory_backend == preset;
        vec![
            line_selectable_option(label, focused, selected),
            line_subtle(description),
        ]
    })
    .chain([line_subtle(
        "Enter keeps the currently selected recommendation.",
    )])
    .collect()
}

fn toggle_choice_lines(cursor: usize, rows: Vec<(String, bool)>) -> Vec<Line<'static>> {
    let mut lines = rows
        .into_iter()
        .enumerate()
        .map(|(index, (label, selected))| line_toggle_option(&label, cursor == index, selected))
        .collect::<Vec<_>>();
    lines.push(line_subtle(
        "Use ↑/↓ to move, ←/→ to toggle, and Enter to continue.",
    ));
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
        SetupScreen::Complete => "The setup is ready.",
    }
}

pub fn setup_controls(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "Enter skip boot | q quit",
        SetupScreen::Detection => "Enter continue | q quit",
        SetupScreen::AskProfile | SetupScreen::AskMemory => {
            "←/→ choose | Enter continue | Backspace back | q quit"
        }
        SetupScreen::AskTools | SetupScreen::AskSkills | SetupScreen::AskMcps => {
            "↑/↓ move | ←/→ toggle | Enter continue | Backspace back | q quit"
        }
        SetupScreen::Confirm => "Enter confirm | Backspace back | q quit",
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
        "OpenAgents> Welcome. I’m going to keep one OpenAgents setup aligned across your tools, skills, MCP servers, and memory.".to_string(),
    ];

    if app.existing_control_plane {
        lines.push("OpenAgents> I found an existing setup, so I only asked about the gaps that still needed attention.".to_string());
    } else {
        lines.push("OpenAgents> I did not find a saved OpenAgents setup yet, so I prepared a starter setup.".to_string());
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
            "OpenAgents> I’m ready to write the setup for `{}`.",
            app.selection.workspace_name
        )],
    }
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

fn line_cursor_option(text: &str, focused: bool) -> Line<'static> {
    let prefix = if focused { "› " } else { "  " };
    let style = if focused {
        Style::default().fg(LIME).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(IVORY)
    };
    Line::from(vec![
        Span::styled(
            prefix,
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), style),
    ])
}

fn line_selectable_option(text: &str, focused: bool, selected: bool) -> Line<'static> {
    let marker = if selected { "●" } else { "○" };
    let style = if focused {
        Style::default().fg(LIME).add_modifier(Modifier::BOLD)
    } else if selected {
        Style::default().fg(TEAL).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(IVORY)
    };
    Line::from(vec![
        Span::styled(if focused { "› " } else { "  " }, Style::default().fg(TEAL)),
        Span::styled(format!("{marker} "), Style::default().fg(LIME)),
        Span::styled(text.to_string(), style),
    ])
}

fn line_toggle_option(text: &str, focused: bool, enabled: bool) -> Line<'static> {
    let label = if enabled {
        format!("[on] {text}")
    } else {
        format!("[off] {text}")
    };
    let style = if focused {
        Style::default().fg(LIME).add_modifier(Modifier::BOLD)
    } else if enabled {
        Style::default().fg(TEAL)
    } else {
        Style::default().fg(IVORY)
    };
    Line::from(vec![
        Span::styled(if focused { "› " } else { "  " }, Style::default().fg(TEAL)),
        Span::styled(label, style),
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

fn profile_index(preset: ProfilePreset) -> usize {
    match preset {
        ProfilePreset::PersonalClient => 0,
        ProfilePreset::TeamWorkspace => 1,
        ProfilePreset::ProjectSandbox => 2,
    }
}

fn profile_preset_from_cursor(index: usize) -> ProfilePreset {
    match index {
        1 => ProfilePreset::TeamWorkspace,
        2 => ProfilePreset::ProjectSandbox,
        _ => ProfilePreset::PersonalClient,
    }
}

fn memory_index(preset: MemoryBackendPreset) -> usize {
    match preset {
        MemoryBackendPreset::Filesystem => 0,
        MemoryBackendPreset::Cortex => 1,
    }
}

fn memory_preset_from_cursor(index: usize) -> MemoryBackendPreset {
    match index {
        1 => MemoryBackendPreset::Cortex,
        _ => MemoryBackendPreset::Filesystem,
    }
}

fn wrap_cursor(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (current + 1) % len
    } else if current == 0 {
        len - 1
    } else {
        current - 1
    }
}

fn home_actions() -> [HomeAction; 5] {
    [
        HomeAction::Sync,
        HomeAction::Doctor,
        HomeAction::History,
        HomeAction::Setup,
        HomeAction::Exit,
    ]
}

fn home_action_label(action: HomeAction) -> &'static str {
    match action {
        HomeAction::Sync => "Sync managed outputs",
        HomeAction::Doctor => "Inspect current health",
        HomeAction::History => "Review setup history",
        HomeAction::Setup => "Re-run guided setup",
        HomeAction::Exit => "Exit",
    }
}

fn home_intro_lines(
    app: &HomeApp,
    control: &ControlPlane,
    cwd: &Path,
    profile: &openagents_core::ResolvedProfile,
    report: &DetectionReport,
) -> Vec<Line<'static>> {
    let mut lines = vec![line_assistant(
        app.intro.as_deref().unwrap_or(
            "Your setup is healthy. I can sync your managed outputs, inspect drift, review setup history, or reopen guided setup.",
        ),
    )];
    lines.push(line_assistant(format!(
        "You are attached to `{}` on this project, with {} managed tool(s).",
        profile.name,
        profile.tools.len()
    )));
    lines.push(line_subtle(format!(
        "project  {}  |  config root  {}",
        cwd.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("current-folder"),
        control.root.display()
    )));
    lines.push(line_subtle(format!(
        "tools seen  {}  |  memory  {}",
        report
            .detections
            .iter()
            .map(|item| item.tool.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        profile.memory.provider
    )));
    lines
}

fn home_action_lines(app: &HomeApp) -> Vec<Line<'static>> {
    home_actions()
        .into_iter()
        .enumerate()
        .map(|(index, action)| {
            line_cursor_option(home_action_label(action), app.action_index == index)
        })
        .collect()
}

fn sync_result_lines(summary: &SyncSummary) -> Vec<Line<'static>> {
    let mut lines = vec![
        line_accent(format!("profile      {}", summary.profile_name)),
        line_plain(format!("managed root {}", summary.managed_root.display())),
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

fn doctor_result_lines(
    _control: &ControlPlane,
    _cwd: &Path,
    profile: &openagents_core::ResolvedProfile,
    report: &DetectionReport,
) -> Vec<Line<'static>> {
    vec![
        line_accent(format!("profile      {}", profile.name)),
        line_plain(format!("memory       {}", profile.memory.provider)),
        line_plain(format!("skills       {}", join_or_none(&profile.skills))),
        line_plain(format!(
            "mcp          {}",
            join_or_none(&profile.mcp_servers)
        )),
        line_plain(format!(
            "detected     {}",
            report
                .detections
                .iter()
                .map(|item| item.tool.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    ]
}

fn history_result_lines(history: &str) -> Vec<Line<'static>> {
    history
        .lines()
        .map(|line| line_plain(line.to_string()))
        .collect()
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
        assert!(setup_controls(SetupScreen::AskSkills).contains("toggle"));
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
    fn controls_use_arrow_navigation_instead_of_numbered_choices() {
        assert!(setup_controls(SetupScreen::AskProfile).contains("←/→"));
        assert!(setup_controls(SetupScreen::AskTools).contains("↑/↓"));
        assert!(setup_controls(SetupScreen::AskTools).contains("←/→"));
        assert!(!setup_controls(SetupScreen::AskTools).contains("1-3"));
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

    #[test]
    fn hero_scene_uses_unicode_pixel_art_instead_of_ascii_console_lines() {
        let hero = hero_lines(HeroState::Listening, "Test", 0)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(hero.contains("▀") || hero.contains("▄") || hero.contains("█"));
        assert!(!hero.contains("[]  []"));
        assert!(!hero.contains(".--|"));
    }
}
