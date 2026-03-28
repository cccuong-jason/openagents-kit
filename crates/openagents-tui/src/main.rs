use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod detection;
mod setup;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use detection::{DetectionReport, detect_tools_in_home};
use openagents_adapters::{render_adapter_output, write_adapter_output};
use openagents_core::{ToolKind, WorkspaceManifest};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use setup::{SetupSelection, recommended_selection, selection_to_manifest, write_manifest};

const BOOT_TICKS: u16 = 8;
const TEAL: Color = Color::Rgb(79, 212, 201);
const LIME: Color = Color::Rgb(185, 255, 102);
const SLATE: Color = Color::Rgb(123, 151, 166);
const IVORY: Color = Color::Rgb(223, 237, 232);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MascotState {
    Scanning,
    Listening,
    Ready,
}

#[derive(Debug, Parser)]
#[command(
    name = "openagents-kit",
    version,
    about = "Cross-tool AI workspace bootstrap."
)]
struct Cli {
    #[arg(long, global = true, default_value = "workspace.yaml")]
    manifest: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init {
        #[arg(long, default_value = ".")]
        output_root: PathBuf,
        #[arg(long, default_value = "starter")]
        workspace_name: String,
    },
    Apply {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        tool: Option<ToolArg>,
        #[arg(long, default_value = "generated")]
        output_root: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    Doctor {
        #[arg(long)]
        profile: Option<String>,
    },
    Adapters,
    Memory {
        #[arg(long)]
        profile: String,
        #[arg(long, default_value = "text")]
        format: MemoryFormatArg,
        #[arg(long)]
        ensure: bool,
    },
    Sync {
        #[arg(long)]
        profile: String,
        #[arg(long, default_value = "generated")]
        output_root: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    Setup {
        #[arg(long, default_value = "generated")]
        output_root: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    Tui,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ToolArg {
    Codex,
    Claude,
    Gemini,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum MemoryFormatArg {
    Text,
    Json,
}

impl ToolArg {
    fn into_tool_kind(self) -> ToolKind {
        match self {
            Self::Codex => ToolKind::Codex,
            Self::Claude => ToolKind::Claude,
            Self::Gemini => ToolKind::Gemini,
        }
    }
}

impl std::fmt::Display for ToolArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.into_tool_kind().fmt(f)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupScreen {
    Boot,
    Detection,
    AskProfile,
    AskMemory,
    AskTools,
    Confirm,
    Complete,
}

struct SetupApp {
    report: DetectionReport,
    selection: SetupSelection,
    screen: SetupScreen,
    tool_cursor: usize,
    boot_tick: u16,
    status: String,
}

impl SetupApp {
    fn new(report: DetectionReport, selection: SetupSelection) -> Self {
        Self {
            report,
            selection,
            screen: SetupScreen::Boot,
            tool_cursor: 0,
            boot_tick: 0,
            status: boot_loading_message(0),
        }
    }

    fn advance_boot(&mut self) {
        if self.screen != SetupScreen::Boot {
            return;
        }

        self.boot_tick = self.boot_tick.saturating_add(1);
        self.status = boot_loading_message(self.boot_tick.into());
        if self.boot_tick < BOOT_TICKS {
            return;
        }

        self.screen = SetupScreen::Detection;
        self.status = "Press Enter and I will walk you through the setup.".to_string();
    }

    fn advance_conversation(&mut self) {
        self.screen = match self.screen {
            SetupScreen::Boot => SetupScreen::Detection,
            SetupScreen::Detection => SetupScreen::AskProfile,
            SetupScreen::AskProfile => SetupScreen::AskMemory,
            SetupScreen::AskMemory => SetupScreen::AskTools,
            SetupScreen::AskTools => SetupScreen::Confirm,
            SetupScreen::Confirm => SetupScreen::Confirm,
            SetupScreen::Complete => SetupScreen::Complete,
        };
        self.status = screen_status(self.screen).to_string();
    }

    fn previous_screen(&mut self) {
        self.screen = match self.screen {
            SetupScreen::AskMemory => SetupScreen::AskProfile,
            SetupScreen::AskTools => SetupScreen::AskMemory,
            SetupScreen::Confirm => SetupScreen::AskTools,
            other => other,
        };
        self.status = screen_status(self.screen).to_string();
    }

    fn move_tool_next(&mut self) {
        self.tool_cursor = (self.tool_cursor + 1) % tool_order().len();
        self.status = screen_status(self.screen).to_string();
    }

    fn move_tool_previous(&mut self) {
        self.tool_cursor = if self.tool_cursor == 0 {
            tool_order().len() - 1
        } else {
            self.tool_cursor - 1
        };
        self.status = screen_status(self.screen).to_string();
    }

    fn selected_tool(&self) -> ToolKind {
        tool_order()[self.tool_cursor]
    }

    fn toggle_selected_tool(&mut self) {
        let tool = self.selected_tool();
        if let Some(position) = self
            .selection
            .enabled_tools
            .iter()
            .position(|item| *item == tool)
        {
            self.selection.enabled_tools.remove(position);
        } else {
            self.selection.enabled_tools.push(tool);
            self.selection.enabled_tools.sort();
        }
        self.status = screen_status(self.screen).to_string();
    }

    fn cycle_current_choice(&mut self, forward: bool) {
        match self.screen {
            SetupScreen::AskProfile => {
                self.selection.profile_preset = if forward {
                    self.selection.profile_preset.next()
                } else {
                    self.selection.profile_preset.previous()
                };
            }
            SetupScreen::AskMemory => {
                self.selection.memory_backend = if forward {
                    self.selection.memory_backend.next()
                } else {
                    self.selection.memory_backend.previous()
                };
            }
            _ => {}
        }
        self.status = screen_status(self.screen).to_string();
    }
}

fn tool_order() -> [ToolKind; 3] {
    [ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini]
}

fn mascot_art(state: MascotState) -> &'static str {
    match state {
        MascotState::Scanning => {
            r"    _.._.._
 .-'_ || _'-.
/ / | || | \ \
| | | .. | | |
| | |____| | |
|_|  |__|  |_|
 /_/      \_\"
        }
        MascotState::Listening => {
            r"    _.._.._
 .-'_ || _'-.
/ / | || | \ \
| | | == | | |
| | |____| | |
|_|  |__|  |_|
 /_/      \_\"
        }
        MascotState::Ready => {
            r"    _.._.._
 .-'_ || _'-.
/ / | || | \ \
| | | ^^ | | |
| | |____| | |
|_|  |__|  |_|
 /_/      \_\"
        }
    }
}

fn boot_loading_message(tick: usize) -> String {
    let dots = ".".repeat((tick % 3) + 1);
    format!("Scanning local AI tools and workspace hints{dots}")
}

fn assistant_heading(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "I'm scanning this machine for supported AI tools.",
        SetupScreen::Detection => {
            "I found your local AI tool footprint and prepared a starting point."
        }
        SetupScreen::AskProfile => "I recommend a Personal Client workspace to start.",
        SetupScreen::AskMemory => {
            "I recommend filesystem memory so you can inspect everything locally first."
        }
        SetupScreen::AskTools => {
            "I found a tool set I can turn into starter outputs for this workspace."
        }
        SetupScreen::Confirm => "I'm ready to generate your workspace now.",
        SetupScreen::Complete => "I finished the setup and wrote your starter files.",
    }
}

fn screen_status(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "I am still scanning. Press Enter if you want to skip ahead.",
        SetupScreen::Detection => "Press Enter and I will ask the first setup question.",
        SetupScreen::AskProfile | SetupScreen::AskMemory => {
            "Use Left and Right to switch options, then press Enter to continue."
        }
        SetupScreen::AskTools => {
            "Use Up and Down to choose a tool, Space to toggle it, then Enter to continue."
        }
        SetupScreen::Confirm => "Press Enter and I will generate the workspace.",
        SetupScreen::Complete => "Review the output, then press Enter to exit.",
    }
}

fn setup_controls(screen: SetupScreen) -> &'static str {
    match screen {
        SetupScreen::Boot => "Enter skip scan | q quit",
        SetupScreen::Detection => "Enter continue | q quit",
        SetupScreen::AskProfile | SetupScreen::AskMemory => {
            "Left/right switch option | Enter continue | q quit"
        }
        SetupScreen::AskTools => {
            "Up/down choose tool | Space toggle | Enter continue | Backspace back | q quit"
        }
        SetupScreen::Confirm => "Enter generate workspace | Backspace back | q quit",
        SetupScreen::Complete => "Enter or q exit",
    }
}

fn progress_labels(screen: SetupScreen) -> [(&'static str, bool); 4] {
    match screen {
        SetupScreen::Boot | SetupScreen::Detection => [
            ("Scan", true),
            ("Profile", false),
            ("Memory", false),
            ("Generate", false),
        ],
        SetupScreen::AskProfile => [
            ("Scan", false),
            ("Profile", true),
            ("Memory", false),
            ("Generate", false),
        ],
        SetupScreen::AskMemory | SetupScreen::AskTools => [
            ("Scan", false),
            ("Profile", false),
            ("Memory", true),
            ("Generate", false),
        ],
        SetupScreen::Confirm | SetupScreen::Complete => [
            ("Scan", false),
            ("Profile", false),
            ("Memory", false),
            ("Generate", true),
        ],
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init {
            output_root,
            workspace_name,
        }) => init_workspace(&output_root, &workspace_name),
        Some(Commands::Apply {
            profile,
            tool,
            output_root,
            dry_run,
        }) => apply_profile(&cli.manifest, &profile, tool, &output_root, dry_run),
        Some(Commands::Sync {
            profile,
            output_root,
            dry_run,
        }) => apply_profile(&cli.manifest, &profile, None, &output_root, dry_run),
        Some(Commands::Doctor { profile }) => doctor(&cli.manifest, profile.as_deref()),
        Some(Commands::Adapters) => list_adapters(),
        Some(Commands::Memory {
            profile,
            format,
            ensure,
        }) => memory(&cli.manifest, &profile, format, ensure),
        Some(Commands::Setup {
            output_root,
            dry_run,
        }) => run_setup(&cli.manifest, &output_root, dry_run),
        Some(Commands::Tui) | None => run_tui(&cli.manifest),
    }
}

fn load_manifest(path: &Path) -> Result<WorkspaceManifest> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest at {}", path.display()))?;
    WorkspaceManifest::from_yaml_str(&contents).context("failed to parse manifest")
}

fn init_workspace(output_root: &Path, workspace_name: &str) -> Result<()> {
    fs::create_dir_all(output_root)
        .with_context(|| format!("failed to create {}", output_root.display()))?;
    let manifest = format!(
        "version: 1\nworkspace: {workspace_name}\nprofiles:\n  personal-client:\n    description: Personal client profile.\n    memory:\n      provider: cortex\n      endpoint: https://personal.example.com\n      scope: client\n    tools:\n      codex:\n        enabled: true\n        guidance_packs:\n          - shared-memory\n      claude:\n        enabled: true\n        guidance_packs:\n          - shared-memory\n      gemini:\n        enabled: true\n        guidance_packs:\n          - shared-memory\n"
    );
    fs::write(output_root.join("workspace.yaml"), manifest)
        .with_context(|| format!("failed to write manifest into {}", output_root.display()))?;
    Ok(())
}

fn apply_profile(
    manifest_path: &Path,
    profile_name: &str,
    tool: Option<ToolArg>,
    output_root: &Path,
    dry_run: bool,
) -> Result<()> {
    let manifest = load_manifest(manifest_path)?;
    let profile = manifest
        .resolve_profile(profile_name)
        .with_context(|| format!("failed to resolve profile `{profile_name}`"))?;

    let tools: Vec<ToolKind> = if let Some(tool) = tool {
        vec![tool.into_tool_kind()]
    } else {
        profile.tools.keys().copied().collect()
    };

    for tool in tools {
        let rendered = render_adapter_output(tool, &manifest.workspace, &profile)?;
        if dry_run {
            println!("--- {} ---\n{rendered}", tool);
        } else {
            write_adapter_output(output_root, tool, &rendered)?;
        }
    }

    Ok(())
}

fn doctor(manifest_path: &Path, profile_name: Option<&str>) -> Result<()> {
    let manifest = load_manifest(manifest_path)?;
    println!("workspace: {}", manifest.workspace);
    println!("profiles: {}", manifest.profiles.len());

    if let Some(profile_name) = profile_name {
        let resolved = manifest.resolve_profile(profile_name)?;
        println!("memory provider: {}", resolved.memory.provider);
        println!("memory endpoint: {}", resolved.memory.endpoint);
        println!("tools: {}", resolved.tools.len());
    }

    Ok(())
}

fn list_adapters() -> Result<()> {
    println!("supported adapters:");
    println!("- codex");
    println!("- claude");
    println!("- gemini");
    Ok(())
}

fn memory(
    manifest_path: &Path,
    profile_name: &str,
    format: MemoryFormatArg,
    ensure: bool,
) -> Result<()> {
    let rendered = render_memory_details(manifest_path, profile_name, format, ensure)?;
    println!("{rendered}");
    Ok(())
}

fn render_memory_details(
    manifest_path: &Path,
    profile_name: &str,
    format: MemoryFormatArg,
    ensure: bool,
) -> Result<String> {
    let manifest = load_manifest(manifest_path)?;
    let resolved = manifest.resolve_profile(profile_name)?;
    let ensured_path = ensure_memory_store(manifest_path, &manifest.workspace, &resolved, ensure)?;

    let rendered = match format {
        MemoryFormatArg::Text => {
            if let Some(path) = ensured_path {
                format!(
                    "memory provider `{}` configured at {}\nseeded local memory store: {}",
                    resolved.memory.provider,
                    resolved.memory.endpoint,
                    path.display()
                )
            } else {
                format!(
                    "memory provider `{}` configured at {}",
                    resolved.memory.provider, resolved.memory.endpoint
                )
            }
        }
        MemoryFormatArg::Json => serde_json::json!({
            "workspace": manifest.workspace,
            "profile": profile_name,
            "provider": resolved.memory.provider,
            "endpoint": resolved.memory.endpoint,
            "scope": resolved.memory.scope,
            "seeded": ensured_path.is_some(),
            "seeded_path": ensured_path.map(|path| path.display().to_string()),
        })
        .to_string(),
    };

    Ok(rendered)
}

fn ensure_memory_store(
    manifest_path: &Path,
    workspace_name: &str,
    profile: &openagents_core::ResolvedProfile,
    ensure: bool,
) -> Result<Option<PathBuf>> {
    if !ensure || profile.memory.provider != "filesystem" {
        return Ok(None);
    }

    let base_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let endpoint_path = PathBuf::from(&profile.memory.endpoint);
    let absolute_path = if endpoint_path.is_absolute() {
        endpoint_path
    } else {
        base_dir.join(endpoint_path)
    };

    fs::create_dir_all(&absolute_path).with_context(|| {
        format!(
            "failed to create memory store at {}",
            absolute_path.display()
        )
    })?;
    fs::write(
        absolute_path.join("README.md"),
        format!(
            "# OpenAgents Memory Store\n\nWorkspace: {workspace_name}\nProfile: {}\nProvider: {}\nEndpoint: {}\n",
            profile.name, profile.memory.provider, profile.memory.endpoint
        ),
    )
    .with_context(|| format!("failed to seed README in {}", absolute_path.display()))?;
    fs::write(
        absolute_path.join("memory.json"),
        serde_json::json!({
            "workspace": workspace_name,
            "profile": profile.name,
            "provider": profile.memory.provider,
            "endpoint": profile.memory.endpoint,
        })
        .to_string(),
    )
    .with_context(|| {
        format!(
            "failed to seed memory metadata in {}",
            absolute_path.display()
        )
    })?;

    Ok(Some(absolute_path))
}

fn run_tui(manifest_path: &Path) -> Result<()> {
    if !manifest_path.exists() {
        return run_setup(manifest_path, Path::new("generated"), false);
    }

    let manifest = load_manifest(manifest_path)?;

    enable_raw_mode()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = loop {
        terminal.draw(|frame| {
            draw_dashboard(frame, &manifest);
        })?;

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && key.code == KeyCode::Char('q')
        {
            break Ok(());
        }
    };

    disable_raw_mode()?;
    result
}

fn run_setup(manifest_path: &Path, output_root: &Path, dry_run: bool) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to resolve current directory")?;
    let home = home_dir()?;
    let report = detect_tools_in_home(&home);
    let selection = recommended_selection(&current_dir, &report.detections);

    if dry_run {
        let manifest = selection_to_manifest(&selection);
        println!("Detected {} supported tools.", report.detections.len());
        println!("{}", serde_yaml::to_string(&manifest)?);
        return Ok(());
    }

    enable_raw_mode()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut app = SetupApp::new(report, selection);

    let result = loop {
        terminal.draw(|frame| {
            draw_setup(frame, &app, manifest_path, output_root);
        })?;

        if !event::poll(Duration::from_millis(250))? {
            if app.screen == SetupScreen::Boot {
                app.advance_boot();
            }
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        match app.screen {
            SetupScreen::Boot => match key.code {
                KeyCode::Enter => {
                    while app.screen == SetupScreen::Boot {
                        app.advance_boot();
                    }
                }
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::Detection => match key.code {
                KeyCode::Enter => app.advance_conversation(),
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::AskProfile | SetupScreen::AskMemory => match key.code {
                KeyCode::Left => app.cycle_current_choice(false),
                KeyCode::Right => app.cycle_current_choice(true),
                KeyCode::Enter => app.advance_conversation(),
                KeyCode::Backspace if app.screen != SetupScreen::AskProfile => {
                    app.previous_screen()
                }
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::AskTools => match key.code {
                KeyCode::Up => app.move_tool_previous(),
                KeyCode::Down => app.move_tool_next(),
                KeyCode::Char(' ') => app.toggle_selected_tool(),
                KeyCode::Enter => app.advance_conversation(),
                KeyCode::Backspace => app.previous_screen(),
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::Confirm => match key.code {
                KeyCode::Enter => match apply_setup(manifest_path, output_root, &app.selection) {
                    Ok(message) => {
                        app.status = message;
                        app.screen = SetupScreen::Complete;
                    }
                    Err(error) => app.status = format!("Could not generate the setup: {error}"),
                },
                KeyCode::Backspace => app.previous_screen(),
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::Complete => match key.code {
                KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
        }
    };

    disable_raw_mode()?;
    result
}

fn apply_setup(
    manifest_path: &Path,
    output_root: &Path,
    selection: &SetupSelection,
) -> Result<String> {
    if selection.enabled_tools.is_empty() {
        anyhow::bail!("choose at least one tool before generating the workspace");
    }

    write_manifest(manifest_path, selection)?;
    let manifest = load_manifest(manifest_path)?;
    let resolved = manifest.resolve_profile(selection.profile_preset.profile_name())?;
    let _ = ensure_memory_store(manifest_path, &manifest.workspace, &resolved, true)?;
    apply_profile(
        manifest_path,
        selection.profile_preset.profile_name(),
        None,
        output_root,
        false,
    )?;

    Ok(format!(
        "Workspace ready. Wrote {} and refreshed generated outputs in {}. Review the files, then press Enter to exit.",
        manifest_path.display(),
        output_root.display()
    ))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .context("failed to resolve the home directory for tool detection")
}

fn draw_dashboard(frame: &mut ratatui::Frame<'_>, manifest: &WorkspaceManifest) {
    let layout = Layout::vertical([
        Constraint::Length(11),
        Constraint::Min(8),
        Constraint::Length(4),
    ])
    .split(frame.area());

    let hero = Layout::horizontal([Constraint::Length(36), Constraint::Min(30)]).split(layout[0]);

    let mascot = Paragraph::new(vec![
        Line::from(Span::styled(
            "OpenAgents Operator Console",
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            mascot_art(MascotState::Ready),
            Style::default().fg(TEAL),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("workspace  {}", manifest.workspace),
            Style::default().fg(IVORY),
        )),
    ])
    .block(panel("Console"))
    .wrap(Wrap { trim: false });
    frame.render_widget(mascot, hero[0]);

    let profile_lines = manifest
        .profiles
        .iter()
        .map(|(name, profile)| {
            let description = profile
                .description
                .clone()
                .unwrap_or_else(|| "No description yet.".to_string());
            Line::from(vec![
                Span::styled(format!("{name:<18}"), Style::default().fg(TEAL)),
                Span::styled(description, Style::default().fg(IVORY)),
            ])
        })
        .collect::<Vec<_>>();

    let summary = Paragraph::new({
        let mut lines = vec![
            Line::from(Span::styled(
                "Ready to sync generated tool starters and inspect workspace health.",
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Profiles",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
        ];
        lines.extend(profile_lines);
        lines
    })
    .block(panel("Workspace Status"))
    .wrap(Wrap { trim: false });
    frame.render_widget(summary, hero[1]);

    let profiles = manifest
        .profiles
        .iter()
        .map(|(name, profile)| {
            let tools = profile
                .tools
                .keys()
                .map(|tool| tool.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            ListItem::new(format!(
                "{name} | memory: {} | tools: {}",
                profile.memory.provider, tools
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(profiles).block(panel("Active Profiles")),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Press q to exit. Use `openagents-kit setup` when you want OpenAgents to re-scan local tools.",
                Style::default().fg(SLATE),
            )),
            Line::from(Span::styled(
                "The console stays cool-toned and explicit so the next action is always visible.",
                Style::default().fg(IVORY),
            )),
        ])
        .block(panel("Next Actions"))
        .wrap(Wrap { trim: false }),
        layout[2],
    );
}

fn draw_setup(
    frame: &mut ratatui::Frame<'_>,
    app: &SetupApp,
    manifest_path: &Path,
    output_root: &Path,
) {
    let layout = Layout::vertical([
        Constraint::Length(13),
        Constraint::Min(10),
        Constraint::Length(5),
    ])
    .split(frame.area());
    let hero = Layout::horizontal([Constraint::Length(38), Constraint::Min(30)]).split(layout[0]);
    let middle = Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(layout[1]);

    let mascot_state = match app.screen {
        SetupScreen::Boot => MascotState::Scanning,
        SetupScreen::Complete => MascotState::Ready,
        _ => MascotState::Listening,
    };
    let progress = progress_labels(app.screen)
        .into_iter()
        .enumerate()
        .flat_map(|(index, (label, active))| {
            let mut spans = Vec::new();
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            let style = if active {
                Style::default().fg(LIME).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(SLATE)
            };
            spans.push(Span::styled(format!("[{}] {label}", index + 1), style));
            spans
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "OpenAgents First Run",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                mascot_art(mascot_state),
                Style::default().fg(TEAL),
            )),
            Line::from(""),
            Line::from(Span::styled(
                assistant_heading(app.screen),
                Style::default().fg(IVORY),
            )),
            Line::from(progress),
            Line::from(Span::styled(
                format!("manifest target  {}", manifest_path.display()),
                Style::default().fg(SLATE),
            )),
            Line::from(Span::styled(
                format!("detected tools   {}", app.report.detections.len()),
                Style::default().fg(SLATE),
            )),
        ])
        .block(panel("Welcome"))
        .wrap(Wrap { trim: false }),
        hero[0],
    );

    frame.render_widget(
        Paragraph::new(summary_lines(app, manifest_path, output_root))
            .block(panel(summary_title(app)))
            .wrap(Wrap { trim: false }),
        hero[1],
    );

    frame.render_widget(
        Paragraph::new(focus_lines(app, manifest_path, output_root))
            .block(panel(focus_title(app)))
            .wrap(Wrap { trim: false }),
        middle[0],
    );
    frame.render_widget(
        Paragraph::new(workspace_plan_lines(app, manifest_path, output_root))
            .block(panel("Workspace Plan"))
            .wrap(Wrap { trim: false }),
        middle[1],
    );

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                setup_controls(app.screen),
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(app.status.clone(), Style::default().fg(IVORY))),
        ])
        .block(panel("Controls"))
        .wrap(Wrap { trim: false }),
        layout[2],
    );
}

fn summary_title(app: &SetupApp) -> &'static str {
    match app.screen {
        SetupScreen::Boot => "Scan Status",
        SetupScreen::Detection => "What I Found",
        SetupScreen::AskProfile
        | SetupScreen::AskMemory
        | SetupScreen::AskTools
        | SetupScreen::Confirm => "Current Setup",
        SetupScreen::Complete => "Next Action",
    }
}

fn summary_lines(app: &SetupApp, manifest_path: &Path, output_root: &Path) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![
            Line::from(Span::styled(
                boot_loading_message(app.boot_tick.into()),
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "No input is required yet. OpenAgents is doing the first pass for you.",
                Style::default().fg(SLATE),
            )),
            Line::from(Span::styled(
                "Press Enter if you want to skip the animation and jump to the recommendation.",
                Style::default().fg(SLATE),
            )),
        ],
        SetupScreen::Detection => {
            let mut lines = vec![Line::from(Span::styled(
                format!(
                    "I found {} supported tool footprint{} and prepared a recommended starting point.",
                    app.report.detections.len(),
                    if app.report.detections.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
                Style::default().fg(IVORY),
            ))];
            lines.push(Line::from(""));
            if app.report.detections.is_empty() {
                lines.push(Line::from(Span::styled(
                    "No local tool config was trustworthy enough to import directly, so I prepared safe defaults and a guided recommendation instead.",
                    Style::default().fg(SLATE),
                )));
            } else {
                lines.extend(app.report.detections.iter().map(|item| {
                    Line::from(vec![
                        Span::styled(format!("{:<8}", item.tool), Style::default().fg(TEAL)),
                        Span::styled(item.summary.clone(), Style::default().fg(IVORY)),
                    ])
                }));
            }
            if !app.report.warnings.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Review notes",
                    Style::default().fg(LIME).add_modifier(Modifier::BOLD),
                )));
                lines.extend(app.report.warnings.iter().map(|warning| {
                    Line::from(Span::styled(warning.clone(), Style::default().fg(SLATE)))
                }));
            }
            lines
        }
        SetupScreen::AskProfile
        | SetupScreen::AskMemory
        | SetupScreen::AskTools
        | SetupScreen::Confirm => workspace_plan_lines(app, manifest_path, output_root),
        SetupScreen::Complete => vec![
            Line::from(Span::styled(
                "Review the generated files, then run the doctor command once to confirm the profile.",
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(
                    "Suggested next step  openagents-kit doctor --manifest {} --profile {}",
                    manifest_path.display(),
                    app.selection.profile_preset.profile_name()
                ),
                Style::default().fg(LIME),
            )),
        ],
    }
}

fn focus_title(app: &SetupApp) -> &'static str {
    match app.screen {
        SetupScreen::Boot => "Boot Sequence",
        SetupScreen::Detection => "Recommendation",
        SetupScreen::AskProfile => "Question 1 of 3",
        SetupScreen::AskMemory => "Question 2 of 3",
        SetupScreen::AskTools => "Question 3 of 3",
        SetupScreen::Confirm => "Ready To Generate",
        SetupScreen::Complete => "Generated Outputs",
    }
}

fn focus_lines(app: &SetupApp, manifest_path: &Path, output_root: &Path) -> Vec<Line<'static>> {
    match app.screen {
        SetupScreen::Boot => vec![
            Line::from(Span::styled(
                "OpenAgents will inspect local tool configs, choose a sensible starter profile, and prepare generated outputs.",
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("manifest -> {}", manifest_path.display()),
                Style::default().fg(TEAL),
            )),
            Line::from(Span::styled(
                format!("outputs  -> {}", output_root.display()),
                Style::default().fg(TEAL),
            )),
        ],
        SetupScreen::Detection => vec![
            Line::from(Span::styled(
                "I reviewed your local tool state and prepared a starter workspace. I will keep leading from here with one question at a time.",
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter and I will ask the first setup question.",
                Style::default().fg(LIME).add_modifier(Modifier::BOLD),
            )),
        ],
        SetupScreen::AskProfile => question_profile_lines(app),
        SetupScreen::AskMemory => question_memory_lines(app),
        SetupScreen::AskTools => question_tool_lines(app),
        SetupScreen::Confirm => confirm_lines(app, manifest_path, output_root),
        SetupScreen::Complete => vec![
            Line::from(Span::styled(
                format!("workspace.yaml written to {}", manifest_path.display()),
                Style::default().fg(IVORY),
            )),
            Line::from(Span::styled(
                format!("starter outputs refreshed in {}", output_root.display()),
                Style::default().fg(IVORY),
            )),
            Line::from(Span::styled(
                "OpenAgents also ensured the filesystem memory store when that backend was selected.",
                Style::default().fg(SLATE),
            )),
        ],
    }
}

fn question_profile_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let options = [
        (
            "Personal Client",
            "Best when one client or one solo workspace needs a clean starter.",
            app.selection.profile_preset == setup::ProfilePreset::PersonalClient,
        ),
        (
            "Team Workspace",
            "Best when multiple people will share a longer-lived setup.",
            app.selection.profile_preset == setup::ProfilePreset::TeamWorkspace,
        ),
        (
            "Project Sandbox",
            "Best when you need a lighter project-specific sandbox.",
            app.selection.profile_preset == setup::ProfilePreset::ProjectSandbox,
        ),
    ];
    let mut lines = vec![
        Line::from(Span::styled(
            "I recommend a Personal Client workspace. Keep that, or switch to another starting profile.",
            Style::default().fg(IVORY),
        )),
        Line::from(""),
    ];
    lines.extend(options.into_iter().flat_map(option_lines));
    lines
}

fn option_lines(option: (&'static str, &'static str, bool)) -> [Line<'static>; 3] {
    let style = if option.2 {
        Style::default().fg(LIME).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(IVORY)
    };
    [
        Line::from(Span::styled(
            format!("{} {}", if option.2 { ">" } else { " " }, option.0),
            style,
        )),
        Line::from(Span::styled(option.1, Style::default().fg(SLATE))),
        Line::from(""),
    ]
}

fn question_memory_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let options = [
        (
            "Filesystem",
            "Starts local and seeds a visible memory store under .openagents.",
            app.selection.memory_backend == setup::MemoryBackendPreset::Filesystem,
        ),
        (
            "Cortex",
            "Keeps the config ready for a hosted or shared memory backend later.",
            app.selection.memory_backend == setup::MemoryBackendPreset::Cortex,
        ),
    ];
    let mut lines = vec![
        Line::from(Span::styled(
            "I recommend Filesystem memory so you can inspect everything locally first.",
            Style::default().fg(IVORY),
        )),
        Line::from(""),
    ];
    lines.extend(options.into_iter().flat_map(option_lines));
    lines
}

fn question_tool_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "I found these tools and I am planning to generate starter outputs for them. Keep this set, or adjust it now.",
            Style::default().fg(IVORY),
        )),
        Line::from(""),
    ];
    lines.extend(tool_order().into_iter().flat_map(|tool| {
        let selected = app.selected_tool() == tool;
        let enabled = app.selection.enabled_tools.contains(&tool);
        let prefix = if enabled { "[x]" } else { "[ ]" };
        let label = if selected { ">" } else { " " };
        let style = if selected {
            Style::default().fg(LIME).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(IVORY)
        };
        [
            Line::from(Span::styled(
                format!("{label} {prefix} {:<7} starter output", tool),
                style,
            )),
            Line::from(Span::styled(
                tool_description(tool),
                Style::default().fg(SLATE),
            )),
            Line::from(""),
        ]
    }));
    lines
}

fn confirm_lines(app: &SetupApp, manifest_path: &Path, output_root: &Path) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "I am ready to generate your workspace now. Here is exactly what I will create.",
            Style::default().fg(IVORY),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("workspace.yaml -> {}", manifest_path.display()),
            Style::default().fg(TEAL),
        )),
    ];

    for tool in &app.selection.enabled_tools {
        lines.push(Line::from(Span::styled(
            format!(
                "{} -> {}/{}",
                tool,
                output_root.display(),
                tool_output_path(*tool)
            ),
            Style::default().fg(TEAL),
        )));
    }

    if app.selection.memory_backend == setup::MemoryBackendPreset::Filesystem {
        lines.push(Line::from(Span::styled(
            format!(
                "filesystem memory -> ./.openagents/memory/{}",
                app.selection.workspace_name
            ),
            Style::default().fg(TEAL),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press Enter and I will write the files. Press Backspace if you want to revisit the previous question.",
        Style::default().fg(LIME),
    )));
    lines
}

fn workspace_plan_lines(
    app: &SetupApp,
    manifest_path: &Path,
    output_root: &Path,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!("workspace       {}", app.selection.workspace_name),
            Style::default().fg(TEAL),
        )),
        Line::from(Span::styled(
            format!("profile         {}", app.selection.profile_preset.label()),
            Style::default().fg(IVORY),
        )),
        Line::from(Span::styled(
            format!("memory          {}", app.selection.memory_backend.label()),
            Style::default().fg(IVORY),
        )),
        Line::from(Span::styled(
            "enabled tools",
            Style::default().fg(LIME).add_modifier(Modifier::BOLD),
        )),
    ];

    lines.extend(tool_order().into_iter().map(|tool| {
        let marker = if app.selection.enabled_tools.contains(&tool) {
            "[x]"
        } else {
            "[ ]"
        };
        let style = if app.screen == SetupScreen::AskTools && app.selected_tool() == tool {
            Style::default().fg(LIME).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(IVORY)
        };
        Line::from(Span::styled(format!("{marker} {tool}"), style))
    }));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("manifest        {}", manifest_path.display()),
        Style::default().fg(SLATE),
    )));
    lines.push(Line::from(Span::styled(
        format!("outputs         {}", output_root.display()),
        Style::default().fg(SLATE),
    )));

    if !app.selection.warnings.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "review reminders",
            Style::default().fg(LIME).add_modifier(Modifier::BOLD),
        )));
        lines.extend(
            app.selection.warnings.iter().map(|warning| {
                Line::from(Span::styled(warning.clone(), Style::default().fg(SLATE)))
            }),
        );
    }

    lines
}

fn tool_output_path(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Codex => "codex/config.toml",
        ToolKind::Claude => "claude/CLAUDE.md",
        ToolKind::Gemini => "gemini/GEMINI.md",
    }
}

fn tool_description(tool: ToolKind) -> &'static str {
    match tool {
        ToolKind::Codex => "Generates a merge-ready Codex starter snippet.",
        ToolKind::Claude => "Generates a starter CLAUDE guidance file.",
        ToolKind::Gemini => "Generates a starter GEMINI guidance file.",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use clap::Parser;
    use tempfile::tempdir;

    use crate::detection::{DetectionReport, ToolDetection};
    use crate::setup::{MemoryBackendPreset, ProfilePreset, SetupSelection};
    use crate::{
        Cli, Commands, MemoryFormatArg, SetupApp, SetupScreen, ToolArg, assistant_heading,
        boot_loading_message, setup_controls,
    };
    use openagents_core::ToolKind;

    fn fixture_path() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/basic/workspace.yaml"
        ))
    }

    #[test]
    fn parses_apply_command() {
        let cli = Cli::parse_from([
            "openagents-kit",
            "apply",
            "--manifest",
            "examples/basic/workspace.yaml",
            "--profile",
            "personal-client",
            "--tool",
            "codex",
            "--dry-run",
        ]);

        match cli.command {
            Some(Commands::Apply {
                profile,
                tool,
                dry_run,
                ..
            }) => {
                assert_eq!(profile, "personal-client");
                assert_eq!(tool.expect("tool should parse").to_string(), "codex");
                assert!(dry_run);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_tui_without_subcommand() {
        let cli = Cli::parse_from(["openagents-kit"]);
        assert!(cli.command.is_none());
    }

    fn sample_detection_report() -> DetectionReport {
        DetectionReport {
            detections: vec![ToolDetection {
                tool: ToolKind::Claude,
                evidence_path: PathBuf::from("C:/Users/example/.claude.json"),
                summary: "Claude state found".to_string(),
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn setup_starts_in_boot_and_advances_to_detection_when_detections_exist() {
        let selection = SetupSelection {
            workspace_name: "starter-workspace".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Claude],
            warnings: Vec::new(),
        };
        let mut app = SetupApp::new(sample_detection_report(), selection);

        assert_eq!(app.screen, SetupScreen::Boot);

        for _ in 0..crate::BOOT_TICKS {
            app.advance_boot();
        }

        assert_eq!(app.screen, SetupScreen::Detection);
    }

    #[test]
    fn setup_boot_always_lands_on_detection_before_questions() {
        let selection = SetupSelection {
            workspace_name: "starter-workspace".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: Vec::new(),
            warnings: Vec::new(),
        };
        let mut app = SetupApp::new(DetectionReport::default(), selection);

        for _ in 0..crate::BOOT_TICKS {
            app.advance_boot();
        }

        assert_eq!(app.screen, SetupScreen::Detection);
    }

    #[test]
    fn controls_text_highlights_question_specific_actions() {
        assert_eq!(
            setup_controls(SetupScreen::Detection),
            "Enter continue | q quit"
        );
        assert_eq!(
            setup_controls(SetupScreen::AskTools),
            "Up/down choose tool | Space toggle | Enter continue | Backspace back | q quit"
        );
        assert_eq!(
            setup_controls(SetupScreen::Confirm),
            "Enter generate workspace | Backspace back | q quit"
        );
        assert_eq!(setup_controls(SetupScreen::Complete), "Enter or q exit");
    }

    #[test]
    fn assistant_heading_reads_like_openagents_is_speaking() {
        assert_eq!(
            assistant_heading(SetupScreen::AskMemory),
            "I recommend filesystem memory so you can inspect everything locally first."
        );
    }

    #[test]
    fn boot_loading_message_cycles_through_scanning_copy() {
        assert_eq!(
            boot_loading_message(0),
            "Scanning local AI tools and workspace hints."
        );
        assert_eq!(
            boot_loading_message(1),
            "Scanning local AI tools and workspace hints.."
        );
        assert_eq!(
            boot_loading_message(2),
            "Scanning local AI tools and workspace hints..."
        );
    }

    #[test]
    fn apply_writes_rendered_output() {
        let temp = tempdir().expect("temp dir should exist");
        let output_root = temp.path().join("generated");

        crate::apply_profile(
            &fixture_path(),
            "personal-client",
            Some(ToolArg::Codex),
            &output_root,
            false,
        )
        .expect("apply should succeed");

        let written = fs::read_to_string(output_root.join("codex/config.toml"))
            .expect("codex output should be written");
        assert!(written.contains("Merge this snippet into ~/.codex/config.toml"));
    }

    #[test]
    fn init_writes_starter_manifest() {
        let temp = tempdir().expect("temp dir should exist");

        crate::init_workspace(temp.path(), "starter-workspace").expect("init should succeed");

        let written = fs::read_to_string(temp.path().join("workspace.yaml"))
            .expect("workspace manifest should exist");
        assert!(written.contains("starter-workspace"));
        assert!(written.contains("personal-client"));
    }

    #[test]
    fn memory_command_can_seed_filesystem_memory_and_render_json() {
        let temp = tempdir().expect("temp dir should exist");
        let manifest_path = temp.path().join("workspace.yaml");
        let selection = SetupSelection {
            workspace_name: "starter-workspace".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![openagents_core::ToolKind::Codex],
            warnings: Vec::new(),
        };

        crate::write_manifest(&manifest_path, &selection).expect("manifest write should succeed");

        let rendered = crate::render_memory_details(
            &manifest_path,
            "personal-client",
            MemoryFormatArg::Json,
            true,
        )
        .expect("memory details should render");

        assert!(rendered.contains("\"provider\":\"filesystem\""));
        assert!(rendered.contains("\"seeded\":true"));
        assert!(
            temp.path()
                .join(".openagents/memory/starter-workspace/README.md")
                .exists()
        );
        assert!(
            temp.path()
                .join(".openagents/memory/starter-workspace/memory.json")
                .exists()
        );
    }

    #[test]
    fn apply_setup_writes_outputs_and_seeds_memory_store() {
        let temp = tempdir().expect("temp dir should exist");
        let manifest_path = temp.path().join("workspace.yaml");
        let output_root = temp.path().join("generated");
        let selection = SetupSelection {
            workspace_name: "starter-workspace".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![
                openagents_core::ToolKind::Codex,
                openagents_core::ToolKind::Claude,
                openagents_core::ToolKind::Gemini,
            ],
            warnings: Vec::new(),
        };

        crate::apply_setup(&manifest_path, &output_root, &selection).expect("setup should apply");

        assert!(output_root.join("codex/config.toml").exists());
        assert!(output_root.join("claude/CLAUDE.md").exists());
        assert!(output_root.join("gemini/GEMINI.md").exists());
        assert!(
            temp.path()
                .join(".openagents/memory/starter-workspace/memory.json")
                .exists()
        );
    }
}
