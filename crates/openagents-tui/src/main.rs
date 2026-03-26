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

const PIXEL_MASCOT: &str = r"   .------.
 .`  .--.  `.
 |  | [] |  |
 |  |____|  |
 | .------. |
 | | 0  0 | |
 |_|__/\__|_|
   /_/  \_\";

const EMBER: Color = Color::Rgb(214, 132, 96);
const SKY: Color = Color::Rgb(130, 156, 178);
const MIST: Color = Color::Rgb(196, 193, 187);

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
    Review,
    Guided,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupField {
    ProfilePreset,
    MemoryBackend,
    Codex,
    Claude,
    Gemini,
}

struct SetupApp {
    report: DetectionReport,
    selection: SetupSelection,
    screen: SetupScreen,
    focused_field: usize,
    status: String,
}

impl SetupApp {
    fn new(report: DetectionReport, selection: SetupSelection) -> Self {
        let screen = if report.detections.is_empty() {
            SetupScreen::Guided
        } else {
            SetupScreen::Review
        };

        Self {
            report,
            selection,
            screen,
            focused_field: 0,
            status: "Press Enter to generate files or g to refine the setup.".to_string(),
        }
    }

    fn fields() -> [SetupField; 5] {
        [
            SetupField::ProfilePreset,
            SetupField::MemoryBackend,
            SetupField::Codex,
            SetupField::Claude,
            SetupField::Gemini,
        ]
    }

    fn selected_field(&self) -> SetupField {
        Self::fields()[self.focused_field]
    }

    fn move_next(&mut self) {
        self.focused_field = (self.focused_field + 1) % Self::fields().len();
    }

    fn move_previous(&mut self) {
        self.focused_field = if self.focused_field == 0 {
            Self::fields().len() - 1
        } else {
            self.focused_field - 1
        };
    }

    fn toggle_selected_tool(&mut self) {
        let tool = match self.selected_field() {
            SetupField::Codex => Some(ToolKind::Codex),
            SetupField::Claude => Some(ToolKind::Claude),
            SetupField::Gemini => Some(ToolKind::Gemini),
            _ => None,
        };

        if let Some(tool) = tool {
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
        }
    }

    fn cycle_right(&mut self) {
        match self.selected_field() {
            SetupField::ProfilePreset => {
                self.selection.profile_preset = self.selection.profile_preset.next();
            }
            SetupField::MemoryBackend => {
                self.selection.memory_backend = self.selection.memory_backend.next();
            }
            SetupField::Codex | SetupField::Claude | SetupField::Gemini => {
                self.toggle_selected_tool()
            }
        }
    }

    fn cycle_left(&mut self) {
        match self.selected_field() {
            SetupField::ProfilePreset => {
                self.selection.profile_preset = self.selection.profile_preset.previous();
            }
            SetupField::MemoryBackend => {
                self.selection.memory_backend = self.selection.memory_backend.previous();
            }
            SetupField::Codex | SetupField::Claude | SetupField::Gemini => {
                self.toggle_selected_tool()
            }
        }
    }
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
        Some(Commands::Memory { profile }) => memory(&cli.manifest, &profile),
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

fn memory(manifest_path: &Path, profile_name: &str) -> Result<()> {
    let manifest = load_manifest(manifest_path)?;
    let resolved = manifest.resolve_profile(profile_name)?;
    println!(
        "memory provider `{}` configured at {}",
        resolved.memory.provider, resolved.memory.endpoint
    );
    Ok(())
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
            draw_setup(frame, &app, manifest_path);
        })?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        match app.screen {
            SetupScreen::Review => match key.code {
                KeyCode::Enter => match apply_setup(manifest_path, output_root, &app.selection) {
                    Ok(message) => {
                        app.status = message;
                        app.screen = SetupScreen::Complete;
                    }
                    Err(error) => app.status = format!("Could not generate the setup: {error}"),
                },
                KeyCode::Char('g') => {
                    app.screen = SetupScreen::Guided;
                    app.status =
                        "Guided mode unlocked. Use arrows to refine the setup.".to_string();
                }
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                _ => {}
            },
            SetupScreen::Guided => match key.code {
                KeyCode::Up => app.move_previous(),
                KeyCode::Down => app.move_next(),
                KeyCode::Left => app.cycle_left(),
                KeyCode::Right => app.cycle_right(),
                KeyCode::Char(' ') => app.toggle_selected_tool(),
                KeyCode::Char('r') => {
                    app.selection = recommended_selection(&current_dir, &app.report.detections);
                    app.status =
                        "Recommended defaults restored from the detection scan.".to_string();
                }
                KeyCode::Enter => match apply_setup(manifest_path, output_root, &app.selection) {
                    Ok(message) => {
                        app.status = message;
                        app.screen = SetupScreen::Complete;
                    }
                    Err(error) => app.status = format!("Could not generate the setup: {error}"),
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
    apply_profile(
        manifest_path,
        selection.profile_preset.profile_name(),
        None,
        output_root,
        false,
    )?;

    Ok(format!(
        "Wrote {} and refreshed generated outputs in {}. Press Enter to exit.",
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
            "OpenAgents Kit",
            Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(PIXEL_MASCOT, Style::default().fg(EMBER))),
        Line::from(""),
        Line::from(Span::styled(
            format!("workspace  {}", manifest.workspace),
            Style::default().fg(MIST),
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("Console"))
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
                Span::styled(format!("{name:<18}"), Style::default().fg(EMBER)),
                Span::styled(description, Style::default().fg(MIST)),
            ])
        })
        .collect::<Vec<_>>();

    let summary = Paragraph::new({
        let mut lines = vec![
            Line::from(Span::styled(
                "Ready for apply, sync, doctor, and targeted repairs.",
                Style::default().fg(MIST),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Profiles",
                Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
            )),
        ];
        lines.extend(profile_lines);
        lines
    })
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Workspace Status"),
    )
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
        List::new(profiles).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Active Profiles"),
        ),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Press q to exit. Use `openagents-kit setup` to re-scan local tools and refresh generated outputs.",
                Style::default().fg(SKY),
            )),
            Line::from(Span::styled(
                "The terminal layout stays warm and compact so the workspace feels calm, not noisy.",
                Style::default().fg(MIST),
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("Next Actions"))
        .wrap(Wrap { trim: false }),
        layout[2],
    );
}

fn draw_setup(frame: &mut ratatui::Frame<'_>, app: &SetupApp, manifest_path: &Path) {
    let layout = Layout::vertical([
        Constraint::Length(12),
        Constraint::Min(10),
        Constraint::Length(4),
    ])
    .split(frame.area());
    let hero = Layout::horizontal([Constraint::Length(36), Constraint::Min(30)]).split(layout[0]);

    let heading = match app.screen {
        SetupScreen::Review => "Auto-detect found an existing AI tool footprint.",
        SetupScreen::Guided => {
            "Guided setup is ready. Refine the starter profile, memory, and tools."
        }
        SetupScreen::Complete => "Setup written. Your starter workspace is now ready.",
    };
    let detection_count = app.report.detections.len();

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "OpenAgents First Run",
                Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(PIXEL_MASCOT, Style::default().fg(EMBER))),
            Line::from(""),
            Line::from(Span::styled(heading, Style::default().fg(MIST))),
            Line::from(Span::styled(
                format!("manifest target  {}", manifest_path.display()),
                Style::default().fg(SKY),
            )),
            Line::from(Span::styled(
                format!("detected tools  {detection_count}"),
                Style::default().fg(SKY),
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("Welcome"))
        .wrap(Wrap { trim: false }),
        hero[0],
    );

    let detection_lines = if app.report.detections.is_empty() {
        vec![
            Line::from(Span::styled(
                "No supported config files were found, so OpenAgents is preparing a guided starter workspace.",
                Style::default().fg(MIST),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Tip: you can still re-scan later after installing Codex, Claude, or Gemini.",
                Style::default().fg(SKY),
            )),
        ]
    } else {
        let mut lines = app
            .report
            .detections
            .iter()
            .map(|item| {
                Line::from(vec![
                    Span::styled(format!("{:<8}", item.tool), Style::default().fg(EMBER)),
                    Span::styled(item.summary.clone(), Style::default().fg(MIST)),
                ])
            })
            .collect::<Vec<_>>();
        if !app.report.warnings.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Review notes",
                Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
            )));
            lines.extend(app.report.warnings.iter().map(|warning| {
                Line::from(Span::styled(warning.clone(), Style::default().fg(SKY)))
            }));
        }
        lines
    };
    frame.render_widget(
        Paragraph::new(detection_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Detected Footprint"),
            )
            .wrap(Wrap { trim: false }),
        hero[1],
    );

    frame.render_widget(
        Paragraph::new(selection_lines(app))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Proposed Workspace"),
            )
            .wrap(Wrap { trim: false }),
        layout[1],
    );

    let controls = match app.screen {
        SetupScreen::Review => "Enter apply starter | g guided setup | q quit",
        SetupScreen::Guided => {
            "Arrows edit | Space toggle tool | r reset defaults | Enter apply | q quit"
        }
        SetupScreen::Complete => "Enter or q exit",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(controls, Style::default().fg(EMBER))),
            Line::from(Span::styled(app.status.clone(), Style::default().fg(MIST))),
        ])
        .block(Block::default().borders(Borders::ALL).title("Controls"))
        .wrap(Wrap { trim: false }),
        layout[2],
    );
}

fn selection_lines(app: &SetupApp) -> Vec<Line<'static>> {
    let selected_style = Style::default().fg(EMBER).add_modifier(Modifier::BOLD);
    let normal_style = Style::default().fg(MIST);

    let line_style = |field: SetupField| {
        if app.screen == SetupScreen::Guided && app.selected_field() == field {
            selected_style
        } else {
            normal_style
        }
    };

    let tool_line = |field: SetupField, tool: ToolKind, enabled: bool| {
        let marker = if enabled { "[x]" } else { "[ ]" };
        Line::from(Span::styled(
            format!("{marker} {:<7} starter adapter output", tool),
            line_style(field),
        ))
    };

    let mut lines = vec![
        Line::from(Span::styled(
            format!("workspace         {}", app.selection.workspace_name),
            Style::default().fg(SKY),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("profile preset    {}", app.selection.profile_preset.label()),
            line_style(SetupField::ProfilePreset),
        )),
        Line::from(Span::styled(
            format!("memory backend    {}", app.selection.memory_backend.label()),
            line_style(SetupField::MemoryBackend),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "enabled tools",
            Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
        )),
        tool_line(
            SetupField::Codex,
            ToolKind::Codex,
            app.selection.enabled_tools.contains(&ToolKind::Codex),
        ),
        tool_line(
            SetupField::Claude,
            ToolKind::Claude,
            app.selection.enabled_tools.contains(&ToolKind::Claude),
        ),
        tool_line(
            SetupField::Gemini,
            ToolKind::Gemini,
            app.selection.enabled_tools.contains(&ToolKind::Gemini),
        ),
    ];

    if !app.selection.warnings.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "review reminders",
            Style::default().fg(EMBER).add_modifier(Modifier::BOLD),
        )));
        lines.extend(
            app.selection
                .warnings
                .iter()
                .map(|warning| Line::from(Span::styled(warning.clone(), Style::default().fg(SKY)))),
        );
    }

    lines
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use clap::Parser;
    use tempfile::tempdir;

    use crate::{Cli, Commands, ToolArg};

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
        assert!(written.contains("[mcp_servers.cortex]"));
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
}
