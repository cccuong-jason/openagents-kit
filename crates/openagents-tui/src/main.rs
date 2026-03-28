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
    Recommend,
    Guided,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WizardStep {
    ProfilePreset,
    MemoryBackend,
    Tools,
    Review,
}

struct SetupApp {
    report: DetectionReport,
    selection: SetupSelection,
    screen: SetupScreen,
    wizard_step: WizardStep,
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
            wizard_step: WizardStep::ProfilePreset,
            tool_cursor: 0,
            boot_tick: 0,
            status: boot_loading_message(0),
        }
    }

    fn next_screen_after_boot(&self) -> SetupScreen {
        if self.report.detections.is_empty() {
            SetupScreen::Guided
        } else {
            SetupScreen::Recommend
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

        self.screen = self.next_screen_after_boot();
        self.status = match self.screen {
            SetupScreen::Recommend => {
                "OpenAgents found existing tool usage and prepared a recommended setup."
                    .to_string()
            }
            SetupScreen::Guided => {
                "No supported tools were found, so OpenAgents opened a guided setup."
                    .to_string()
            }
            _ => self.status.clone(),
        };
    }

    fn enter_guided(&mut self) {
        self.screen = SetupScreen::Guided;
        self.wizard_step = WizardStep::ProfilePreset;
        self.tool_cursor = 0;
        self.status = guided_status(self.wizard_step).to_string();
    }

    fn previous_step(&mut self) {
        self.wizard_step = match self.wizard_step {
            WizardStep::ProfilePreset => WizardStep::ProfilePreset,
            WizardStep::MemoryBackend => WizardStep::ProfilePreset,
            WizardStep::Tools => WizardStep::MemoryBackend,
            WizardStep::Review => WizardStep::Tools,
        };
        self.status = guided_status(self.wizard_step).to_string();
    }

    fn next_step(&mut self) {
        self.wizard_step = match self.wizard_step {
            WizardStep::ProfilePreset => WizardStep::MemoryBackend,
            WizardStep::MemoryBackend => WizardStep::Tools,
            WizardStep::Tools => WizardStep::Review,
            WizardStep::Review => WizardStep::Review,
        };
        self.status = guided_status(self.wizard_step).to_string();
    }

    fn move_tool_next(&mut self) {
        self.tool_cursor = (self.tool_cursor + 1) % tool_order().len();
        self.status = guided_status(self.wizard_step).to_string();
    }

    fn move_tool_previous(&mut self) {
        self.tool_cursor = if self.tool_cursor == 0 {
            tool_order().len() - 1
        } else {
            self.tool_cursor - 1
        };
        self.status = guided_status(self.wizard_step).to_string();
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
        self.status = guided_status(self.wizard_step).to_string();
    }

    fn cycle_current_choice(&mut self, forward: bool) {
        match self.wizard_step {
            WizardStep::ProfilePreset => {
                self.selection.profile_preset = if forward {
                    self.selection.profile_preset.next()
                } else {
                    self.selection.profile_preset.previous()
                };
            }
            WizardStep::MemoryBackend => {
                self.selection.memory_backend = if forward {
                    self.selection.memory_backend.next()
                } else {
                    self.selection.memory_backend.previous()
                };
            }
            WizardStep::Tools => {}
            WizardStep::Review => {}
        }
        self.status = guided_status(self.wizard_step).to_string();
    }
}

fn tool_order() -> [ToolKind; 3] {
    [ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini]
}

fn mascot_art(state: MascotState) -> &'static str {
    match state {
        MascotState::Scanning => {
            r"    _^_^_
 .` - - `.
 |  ___  |
 | |===| |
 |  |_|  |
 '.___.'"
        }
        MascotState::Ready => {
            r"    _^_^_
 .` ^ ^ `.
 |  ___  |
 | |___| |
 | _| |_ |
 '.___.'"
        }
    }
}

fn boot_loading_message(tick: usize) -> String {
    let dots = ".".repeat((tick % 3) + 1);
    format!("Scanning local AI tools and workspace hints{dots}")
}

fn guided_status(step: WizardStep) -> &'static str {
    match step {
        WizardStep::ProfilePreset => {
            "Choose the workspace style OpenAgents should prepare for this client."
        }
        WizardStep::MemoryBackend => {
            "Choose where the starter memory should live before sharing the workspace."
        }
        WizardStep::Tools => {
            "Choose which tool starter files OpenAgents should generate for this workspace."
        }
        WizardStep::Review => {
            "Review the final setup, then press Enter once to generate the workspace."
        }
    }
}

fn setup_controls(screen: SetupScreen, wizard_step: WizardStep) -> &'static str {
    match screen {
        SetupScreen::Boot => "Enter skip scan | q quit",
        SetupScreen::Recommend => "Enter accept recommendation | Tab guided setup | q quit",
        SetupScreen::Guided => match wizard_step {
            WizardStep::ProfilePreset | WizardStep::MemoryBackend => {
                "Left/right change option | Enter continue | Backspace back | q quit"
            }
            WizardStep::Tools => {
                "Up/down choose tool | Space toggle | Enter continue | Backspace back | q quit"
            }
            WizardStep::Review => "Enter generate files | Backspace back | r reset | q quit",
        },
        SetupScreen::Complete => "Enter or q exit",
    }
}

fn progress_labels(screen: SetupScreen) -> [(&'static str, bool); 4] {
    match screen {
        SetupScreen::Boot => [
            ("Scan", true),
            ("Recommend", false),
            ("Adjust", false),
            ("Generate", false),
        ],
        SetupScreen::Recommend => [
            ("Scan", false),
            ("Recommend", true),
            ("Adjust", false),
            ("Generate", false),
        ],
        SetupScreen::Guided => [
            ("Scan", false),
            ("Recommend", false),
            ("Adjust", true),
            ("Generate", false),
        ],
        SetupScreen::Complete => [
            ("Scan", false),
            ("Recommend", false),
            ("Adjust", false),
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
            SetupScreen::Recommend => match key.code {
                KeyCode::Enter => match apply_setup(manifest_path, output_root, &app.selection) {
                    Ok(message) => {
                        app.status = message;
                        app.screen = SetupScreen::Complete;
                    }
                    Err(error) => app.status = format!("Could not generate the setup: {error}"),
                },
                KeyCode::Tab => {
                    app.enter_guided();
                }
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::Guided => match key.code {
                KeyCode::Left
                    if matches!(
                        app.wizard_step,
                        WizardStep::ProfilePreset | WizardStep::MemoryBackend
                    ) =>
                {
                    app.cycle_current_choice(false)
                }
                KeyCode::Right
                    if matches!(
                        app.wizard_step,
                        WizardStep::ProfilePreset | WizardStep::MemoryBackend
                    ) =>
                {
                    app.cycle_current_choice(true)
                }
                KeyCode::Up if app.wizard_step == WizardStep::Tools => app.move_tool_previous(),
                KeyCode::Down if app.wizard_step == WizardStep::Tools => app.move_tool_next(),
                KeyCode::Char(' ') if app.wizard_step == WizardStep::Tools => {
                    app.toggle_selected_tool()
                }
                KeyCode::Char('r') => {
                    app.selection = recommended_selection(&current_dir, &app.report.detections);
                    app.wizard_step = WizardStep::ProfilePreset;
                    app.tool_cursor = 0;
                    app.status =
                        "Recommended defaults restored. Continue step by step from the top."
                            .to_string();
                }
                KeyCode::Backspace => app.previous_step(),
                KeyCode::Enter => match app.wizard_step {
                    WizardStep::Review => {
                        match apply_setup(manifest_path, output_root, &app.selection) {
                            Ok(message) => {
                                app.status = message;
                                app.screen = SetupScreen::Complete;
                            }
                            Err(error) => {
                                app.status = format!("Could not generate the setup: {error}")
                            }
                        }
                    }
                    _ => app.next_step(),
                },
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
        Line::from(Span::styled(mascot_art(MascotState::Ready), Style::default().fg(TEAL))),
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

    let mascot_state = if app.screen == SetupScreen::Boot {
        MascotState::Scanning
    } else {
        MascotState::Ready
    };
    let heading = match app.screen {
        SetupScreen::Boot => "OpenAgents is scanning your local tool footprint.",
        SetupScreen::Recommend => "OpenAgents prepared a recommendation from what it found.",
        SetupScreen::Guided => "Guided setup is active so you can tune the recommendation step by step.",
        SetupScreen::Complete => "Your starter workspace is ready and OpenAgents has written the files.",
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
            Line::from(Span::styled(heading, Style::default().fg(IVORY))),
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
        Paragraph::new(summary_lines(app, manifest_path))
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
                setup_controls(app.screen, app.wizard_step),
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
        SetupScreen::Boot => "Scanning",
        SetupScreen::Recommend => "Detected Footprint",
        SetupScreen::Guided => "Current Step",
        SetupScreen::Complete => "Next Action",
    }
}

fn summary_lines(app: &SetupApp, manifest_path: &Path) -> Vec<Line<'static>> {
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
        SetupScreen::Recommend => {
            let mut lines = vec![Line::from(Span::styled(
                "OpenAgents found these local tool footprints:",
                Style::default().fg(IVORY),
            ))];
            lines.extend(app.report.detections.iter().map(|item| {
                Line::from(vec![
                    Span::styled(format!("{:<8}", item.tool), Style::default().fg(TEAL)),
                    Span::styled(item.summary.clone(), Style::default().fg(IVORY)),
                ])
            }));
            if !app.report.warnings.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Review notes",
                    Style::default().fg(LIME).add_modifier(Modifier::BOLD),
                )));
                lines.extend(
                    app.report
                        .warnings
                        .iter()
                        .map(|warning| Line::from(Span::styled(warning.clone(), Style::default().fg(SLATE)))),
                );
            }
            lines
        }
        SetupScreen::Guided => vec![
            Line::from(Span::styled(
                guided_status(app.wizard_step),
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("Step: {}", wizard_step_label(app.wizard_step)),
                Style::default().fg(LIME),
            )),
            Line::from(Span::styled(
                "OpenAgents keeps the recommendation visible on the right while you adjust details.",
                Style::default().fg(SLATE),
            )),
        ],
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
        SetupScreen::Boot => "OpenAgents Is Resolving",
        SetupScreen::Recommend => "Recommended Setup",
        SetupScreen::Guided => "Guided Adjustments",
        SetupScreen::Complete => "Generated Outputs",
    }
}

fn focus_lines(
    app: &SetupApp,
    manifest_path: &Path,
    output_root: &Path,
) -> Vec<Line<'static>> {
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
        SetupScreen::Recommend => vec![
            Line::from(Span::styled(
                "Primary action",
                Style::default().fg(LIME).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Press Enter to accept the recommendation and let OpenAgents generate the workspace.",
                Style::default().fg(IVORY),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Need changes?",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Press Tab to adjust profile, memory, or tools one step at a time.",
                Style::default().fg(IVORY),
            )),
        ],
        SetupScreen::Guided => guided_focus_lines(app),
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

fn guided_focus_lines(app: &SetupApp) -> Vec<Line<'static>> {
    match app.wizard_step {
        WizardStep::ProfilePreset => {
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
            options
                .into_iter()
                .flat_map(option_lines)
                .collect::<Vec<_>>()
        }
        WizardStep::MemoryBackend => {
            let options = [
                (
                    "Filesystem",
                    "Starts local and seeds a visible memory store under .openagents.",
                    app.selection.memory_backend == setup::MemoryBackendPreset::Filesystem,
                ),
                (
                    "Cortex",
                    "Keeps the config ready for a hosted/shared memory backend later.",
                    app.selection.memory_backend == setup::MemoryBackendPreset::Cortex,
                ),
            ];
            options
                .into_iter()
                .flat_map(option_lines)
                .collect::<Vec<_>>()
        }
        WizardStep::Tools => tool_order()
            .into_iter()
            .flat_map(|tool| {
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
            })
            .collect(),
        WizardStep::Review => vec![
            Line::from(Span::styled(
                "Everything is ready to generate.",
                Style::default().fg(IVORY),
            )),
            Line::from(Span::styled(
                "Press Enter once to write the manifest, memory store, and generated tool files.",
                Style::default().fg(LIME),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Backspace if you want to revisit the previous step first.",
                Style::default().fg(SLATE),
            )),
        ],
    }
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
        let style = if app.screen == SetupScreen::Guided
            && app.wizard_step == WizardStep::Tools
            && app.selected_tool() == tool
        {
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
            app.selection
                .warnings
                .iter()
                .map(|warning| Line::from(Span::styled(warning.clone(), Style::default().fg(SLATE)))),
        );
    }

    lines
}

fn wizard_step_label(step: WizardStep) -> &'static str {
    match step {
        WizardStep::ProfilePreset => "Profile",
        WizardStep::MemoryBackend => "Memory",
        WizardStep::Tools => "Tools",
        WizardStep::Review => "Review",
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
        Cli, Commands, MemoryFormatArg, SetupApp, SetupScreen, ToolArg, WizardStep,
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
    fn setup_starts_in_boot_and_advances_to_recommend_when_detections_exist() {
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

        assert_eq!(app.screen, SetupScreen::Recommend);
    }

    #[test]
    fn setup_advances_to_guided_when_boot_finishes_without_detections() {
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

        assert_eq!(app.screen, SetupScreen::Guided);
        assert_eq!(app.wizard_step, WizardStep::ProfilePreset);
    }

    #[test]
    fn controls_text_highlights_primary_action_per_screen() {
        assert_eq!(
            setup_controls(SetupScreen::Recommend, WizardStep::ProfilePreset),
            "Enter accept recommendation | Tab guided setup | q quit"
        );
        assert_eq!(
            setup_controls(SetupScreen::Guided, WizardStep::Tools),
            "Up/down choose tool | Space toggle | Enter continue | Backspace back | q quit"
        );
        assert_eq!(
            setup_controls(SetupScreen::Complete, WizardStep::Review),
            "Enter or q exit"
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
