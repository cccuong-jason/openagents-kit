use std::fs;
use std::io::stdout;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use openagents_adapters::{render_adapter_output, write_adapter_output};
use openagents_core::{ToolKind, WorkspaceManifest};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

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
    let manifest = load_manifest(manifest_path)?;

    enable_raw_mode()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = loop {
        terminal.draw(|frame| {
            let layout = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(frame.area());

            let header = Paragraph::new(format!(
                "OpenAgents Kit | workspace: {} | press q to exit",
                manifest.workspace
            ))
            .block(Block::default().borders(Borders::ALL).title("Overview"));
            frame.render_widget(header, layout[0]);

            let profiles = manifest
                .profiles
                .iter()
                .map(|(name, profile)| {
                    let description = profile.description.clone().unwrap_or_default();
                    ListItem::new(format!("{name} - {description}"))
                })
                .collect::<Vec<_>>();
            let profile_list =
                List::new(profiles).block(Block::default().borders(Borders::ALL).title("Profiles"));
            frame.render_widget(profile_list, layout[1]);

            let footer = Paragraph::new("Use `apply`, `sync`, or `doctor` for scripted workflows.")
                .block(Block::default().borders(Borders::ALL).title("Commands"));
            frame.render_widget(footer, layout[2]);
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
