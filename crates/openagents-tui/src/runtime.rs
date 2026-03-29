use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use openagents_adapters::{render_adapter_output, write_adapter_output};
use openagents_core::{
    CatalogItemKind, DeviceOverlay, OpenAgentsConfig, ResolvedProfile, WorkspaceManifest,
};

use crate::catalog::{curated_items, install_catalog_assets};
use crate::control::{ControlPlane, default_config_root, device_name};
use crate::detection::{DetectionReport, detect_tools_in_home};
use crate::setup::{
    SetupSelection, recommended_selection, selection_from_config, selection_to_config,
};
use crate::ui;

#[derive(Debug, Parser)]
#[command(
    name = "openagents-kit",
    version,
    about = "Global control plane for cross-tool AI setup."
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true, hide = true)]
    pub manifest: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Setup {
        #[arg(long)]
        dry_run: bool,
    },
    Sync {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Doctor {
        #[arg(long)]
        profile: Option<String>,
    },
    Memory {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long, default_value = "text")]
        format: MemoryFormatArg,
        #[arg(long)]
        ensure: bool,
    },
    Catalog {
        #[arg(long)]
        kind: Option<CatalogKindArg>,
    },
    Attach {
        #[arg(long)]
        profile: Option<String>,
    },
    Tui,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum MemoryFormatArg {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CatalogKindArg {
    Skill,
    Mcp,
}

impl From<CatalogKindArg> for CatalogItemKind {
    fn from(value: CatalogKindArg) -> Self {
        match value {
            CatalogKindArg::Skill => CatalogItemKind::Skill,
            CatalogKindArg::Mcp => CatalogItemKind::Mcp,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub profile_name: String,
    pub managed_root: PathBuf,
    pub tool_paths: Vec<PathBuf>,
    pub skill_paths: Vec<PathBuf>,
    pub mcp_paths: Vec<PathBuf>,
    pub memory_path: Option<PathBuf>,
}

pub fn dispatch(cli: Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    match cli.command {
        Some(Commands::Setup { dry_run }) => ui::run_setup(
            cli.config.as_deref(),
            cli.manifest.as_deref(),
            &cwd,
            dry_run,
        ),
        Some(Commands::Sync { profile, dry_run }) => sync_command(
            cli.config.as_deref(),
            cli.manifest.as_deref(),
            &cwd,
            profile.as_deref(),
            dry_run,
        ),
        Some(Commands::Doctor { profile }) => doctor_command(
            cli.config.as_deref(),
            cli.manifest.as_deref(),
            &cwd,
            profile.as_deref(),
        ),
        Some(Commands::Memory {
            profile,
            format,
            ensure,
        }) => memory_command(
            cli.config.as_deref(),
            cli.manifest.as_deref(),
            &cwd,
            profile.as_deref(),
            format,
            ensure,
        ),
        Some(Commands::Catalog { kind }) => catalog_command(
            cli.config.as_deref(),
            cli.manifest.as_deref(),
            kind.map(Into::into),
        ),
        Some(Commands::Attach { profile }) => attach_command(
            cli.config.as_deref(),
            cli.manifest.as_deref(),
            &cwd,
            profile.as_deref(),
        ),
        Some(Commands::Tui) | None => {
            ui::run_tui(cli.config.as_deref(), cli.manifest.as_deref(), &cwd)
        }
    }
}

pub fn load_detection_report() -> Result<DetectionReport> {
    Ok(detect_tools_in_home(&home_dir()?))
}

pub fn existing_selection(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
) -> Option<SetupSelection> {
    if let Ok(control) = ControlPlane::load(config_override, manifest_override) {
        return Some(selection_from_config(&control.config));
    }

    let legacy_manifest = manifest_override.map(Path::to_path_buf).or_else(|| {
        let path = cwd.join("workspace.yaml");
        path.exists().then_some(path)
    })?;

    let contents = fs::read_to_string(legacy_manifest).ok()?;
    let manifest = WorkspaceManifest::from_yaml_str(&contents).ok()?;
    let config = OpenAgentsConfig::from_manifest(manifest);
    Some(selection_from_config(&config))
}

pub fn recommended_setup_selection(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
) -> Result<(DetectionReport, SetupSelection)> {
    let report = load_detection_report()?;
    let selection = existing_selection(config_override, manifest_override, cwd)
        .unwrap_or_else(|| recommended_selection(&report));
    Ok((report, selection))
}

pub fn resolve_config_path(config_override: Option<&Path>) -> Result<PathBuf> {
    match config_override {
        Some(path) if path.extension().is_some() => Ok(path.to_path_buf()),
        Some(path) => Ok(path.join("config.yaml")),
        None => Ok(default_config_root()?.join("config.yaml")),
    }
}

pub fn apply_setup(
    config_override: Option<&Path>,
    cwd: &Path,
    selection: &SetupSelection,
) -> Result<SyncSummary> {
    let config_path = resolve_config_path(config_override)?;
    let root = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let existing = ControlPlane::load(config_override, None).ok();
    let mut control = ControlPlane {
        root: root.clone(),
        config_path: config_path.clone(),
        overlay_path: root.join("device.yaml"),
        attachments_path: root.join("attachments.yaml"),
        origin: crate::control::ControlPlaneOrigin::GlobalConfig,
        config: selection_to_config(selection),
        overlay: existing
            .as_ref()
            .map(|plane| plane.overlay.clone())
            .unwrap_or_else(|| DeviceOverlay::new(device_name())),
        attachments: existing
            .as_ref()
            .map(|plane| plane.attachments.clone())
            .unwrap_or_else(openagents_core::AttachmentRegistry::new),
    };
    control.attach_current_path(cwd, selection.profile_preset.profile_name());
    control.save()?;
    sync_control_plane(&control, selection.profile_preset.profile_name(), false)
}

pub fn sync_control_plane(
    control: &ControlPlane,
    profile_name: &str,
    dry_run: bool,
) -> Result<SyncSummary> {
    let resolved = control.resolved_profile(profile_name)?;
    let managed_root = control.managed_root();
    let tool_root = managed_root.join("tools");

    let mut tool_paths = Vec::new();
    for (tool, config) in &resolved.tools {
        if !config.enabled {
            continue;
        }

        let rendered = render_adapter_output(*tool, &control.config.workspace_name, &resolved)?;
        let path = tool_root.join(tool.to_string()).join(tool.file_name());
        tool_paths.push(path.clone());
        if !dry_run {
            write_adapter_output(&tool_root, *tool, &rendered)?;
        }
    }

    let catalog_summary = install_catalog_assets(
        &managed_root,
        &resolved.skills,
        &resolved.mcp_servers,
        &control.config.custom_catalog,
        dry_run,
    )?;
    let memory_path = ensure_memory_store(control, &resolved, !dry_run)?;

    if !dry_run {
        fs::create_dir_all(&managed_root)?;
        fs::write(
            managed_root.join("control-plane-export.yaml"),
            serde_yaml::to_string(&control.config)?,
        )?;
    }

    Ok(SyncSummary {
        profile_name: profile_name.to_string(),
        managed_root,
        tool_paths,
        skill_paths: catalog_summary.skill_paths,
        mcp_paths: catalog_summary.mcp_paths,
        memory_path,
    })
}

pub fn ensure_memory_store(
    control: &ControlPlane,
    profile: &ResolvedProfile,
    ensure: bool,
) -> Result<Option<PathBuf>> {
    if !ensure || profile.memory.provider != "filesystem" {
        return Ok(None);
    }

    let absolute_path = PathBuf::from(&profile.memory.endpoint);
    fs::create_dir_all(&absolute_path).with_context(|| {
        format!(
            "failed to create memory store at {}",
            absolute_path.display()
        )
    })?;

    fs::write(
        absolute_path.join("README.md"),
        format!(
            "# OpenAgents Memory Store\n\nWorkspace: {}\nProfile: {}\nProvider: {}\nEndpoint: {}\n",
            control.config.workspace_name,
            profile.name,
            profile.memory.provider,
            profile.memory.endpoint
        ),
    )?;
    fs::write(
        absolute_path.join("memory.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "workspace": control.config.workspace_name,
            "profile": profile.name,
            "provider": profile.memory.provider,
            "endpoint": profile.memory.endpoint,
            "skills": profile.skills,
            "mcp_servers": profile.mcp_servers,
        }))?,
    )?;

    Ok(Some(absolute_path))
}

fn sync_command(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
    explicit_profile: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    let control = ControlPlane::load(config_override, manifest_override)?;
    let profile_name = control.active_profile_name(cwd, explicit_profile);
    let summary = sync_control_plane(&control, &profile_name, dry_run)?;

    println!("workspace: {}", control.config.workspace_name);
    println!("profile: {}", summary.profile_name);
    println!("managed root: {}", summary.managed_root.display());
    println!("tool outputs: {}", join_display_paths(&summary.tool_paths));
    println!("skills: {}", join_display_paths(&summary.skill_paths));
    println!("mcp servers: {}", join_display_paths(&summary.mcp_paths));
    if let Some(memory_path) = &summary.memory_path {
        println!("memory store: {}", memory_path.display());
    }
    Ok(())
}

fn doctor_command(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
    explicit_profile: Option<&str>,
) -> Result<()> {
    let control = ControlPlane::load(config_override, manifest_override)?;
    let profile_name = control.active_profile_name(cwd, explicit_profile);
    let resolved = control.resolved_profile(&profile_name)?;
    let report = load_detection_report()?;

    let missing_tools = resolved
        .tools
        .keys()
        .filter(|tool| !report.detections.iter().any(|item| item.tool == **tool))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let missing_skills = resolved
        .skills
        .iter()
        .filter(|skill| !report.installed_skills.contains(*skill))
        .cloned()
        .collect::<Vec<_>>();
    let missing_mcps = resolved
        .mcp_servers
        .iter()
        .filter(|mcp| !report.installed_mcp_servers.contains(*mcp))
        .cloned()
        .collect::<Vec<_>>();

    println!("workspace: {}", control.config.workspace_name);
    println!("profile: {}", profile_name);
    println!("memory provider: {}", resolved.memory.provider);
    println!("memory endpoint: {}", resolved.memory.endpoint);
    println!(
        "enabled tools: {}",
        resolved
            .tools
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "desired skills: {}",
        if resolved.skills.is_empty() {
            "none".to_string()
        } else {
            resolved.skills.join(", ")
        }
    );
    println!(
        "desired mcp servers: {}",
        if resolved.mcp_servers.is_empty() {
            "none".to_string()
        } else {
            resolved.mcp_servers.join(", ")
        }
    );
    println!(
        "missing tools: {}",
        if missing_tools.is_empty() {
            "none".to_string()
        } else {
            missing_tools.join(", ")
        }
    );
    println!(
        "missing skills: {}",
        if missing_skills.is_empty() {
            "none".to_string()
        } else {
            missing_skills.join(", ")
        }
    );
    println!(
        "missing mcp servers: {}",
        if missing_mcps.is_empty() {
            "none".to_string()
        } else {
            missing_mcps.join(", ")
        }
    );
    println!(
        "memory layer detected on this device: {}",
        if report.has_memory_layer { "yes" } else { "no" }
    );

    Ok(())
}

fn memory_command(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
    explicit_profile: Option<&str>,
    format: MemoryFormatArg,
    ensure: bool,
) -> Result<()> {
    let control = ControlPlane::load(config_override, manifest_override)?;
    let profile_name = control.active_profile_name(cwd, explicit_profile);
    let resolved = control.resolved_profile(&profile_name)?;
    let memory_path = ensure_memory_store(&control, &resolved, ensure)?;

    match format {
        MemoryFormatArg::Text => {
            if let Some(path) = &memory_path {
                println!(
                    "memory provider `{}` configured at {}\nseeded local memory store: {}",
                    resolved.memory.provider,
                    resolved.memory.endpoint,
                    path.display()
                );
            } else {
                println!(
                    "memory provider `{}` configured at {}",
                    resolved.memory.provider, resolved.memory.endpoint
                );
            }
        }
        MemoryFormatArg::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "workspace": control.config.workspace_name,
                "profile": profile_name,
                "provider": resolved.memory.provider,
                "endpoint": resolved.memory.endpoint,
                "seeded_path": memory_path.map(|path| path.display().to_string()),
            }))?
        ),
    }

    Ok(())
}

fn catalog_command(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    kind: Option<CatalogItemKind>,
) -> Result<()> {
    let custom_catalog = ControlPlane::load(config_override, manifest_override)
        .map(|plane| plane.config.custom_catalog)
        .unwrap_or_default();

    for item in curated_items()
        .iter()
        .filter(|item| kind.is_none() || Some(item.kind) == kind)
    {
        println!(
            "[curated] {} | {} | {} | tools: {}",
            label_for_kind(item.kind),
            item.id,
            item.description,
            item.supported_tools
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    for (id, item) in custom_catalog
        .iter()
        .filter(|(_, item)| kind.is_none() || Some(item.kind) == kind)
    {
        println!(
            "[custom] {} | {} | {} | tools: {}",
            label_for_kind(item.kind),
            id,
            item.description,
            item.supported_tools
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}

fn attach_command(
    config_override: Option<&Path>,
    manifest_override: Option<&Path>,
    cwd: &Path,
    profile: Option<&str>,
) -> Result<()> {
    let mut control = ControlPlane::load(config_override, manifest_override)?;
    let profile_name = profile
        .unwrap_or(&control.config.default_profile)
        .to_string();
    control.attach_current_path(cwd, &profile_name);
    control.save()?;
    println!("attached {} to profile {}", cwd.display(), profile_name);
    Ok(())
}

fn join_display_paths(paths: &[PathBuf]) -> String {
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

fn label_for_kind(kind: CatalogItemKind) -> &'static str {
    match kind {
        CatalogItemKind::Skill => "skill",
        CatalogItemKind::Mcp => "mcp",
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .context("failed to determine home directory")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{apply_setup, sync_control_plane};
    use crate::control::{ControlPlane, device_name};
    use crate::setup::{MemoryBackendPreset, ProfilePreset, SetupSelection, selection_to_config};
    use openagents_core::{AttachmentRegistry, DeviceOverlay, OpenAgentsConfig, WorkspaceManifest};

    #[test]
    fn apply_setup_writes_global_config_and_managed_outputs() {
        let temp = tempdir().expect("temp dir should exist");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("repo dir should exist");
        let config_root = temp.path().join("openagents-config");
        let selection = SetupSelection {
            workspace_name: "openagents-home".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![openagents_core::ToolKind::Codex],
            selected_skills: vec!["shared-memory".to_string()],
            selected_mcp_servers: vec!["filesystem-memory".to_string()],
            warnings: vec![],
        };

        let summary =
            apply_setup(Some(&config_root), &repo, &selection).expect("setup should apply");

        assert!(config_root.join("config.yaml").exists());
        assert!(config_root.join("attachments.yaml").exists());
        assert_eq!(summary.tool_paths.len(), 1);
        assert!(summary.skill_paths[0].exists());
        assert!(summary.mcp_paths[0].exists());
        assert!(summary.memory_path.expect("memory path").exists());
    }

    #[test]
    fn sync_writes_export_and_memory_store() {
        let temp = tempdir().expect("temp dir should exist");
        let root = temp.path().join("config");
        fs::create_dir_all(&root).expect("config root should exist");

        let selection = SetupSelection {
            workspace_name: "openagents-home".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![openagents_core::ToolKind::Claude],
            selected_skills: vec!["shared-memory".to_string()],
            selected_mcp_servers: vec!["filesystem-memory".to_string()],
            warnings: vec![],
        };
        let control = ControlPlane {
            root: root.clone(),
            config_path: root.join("config.yaml"),
            overlay_path: root.join("device.yaml"),
            attachments_path: root.join("attachments.yaml"),
            origin: crate::control::ControlPlaneOrigin::GlobalConfig,
            config: selection_to_config(&selection),
            overlay: DeviceOverlay::new(device_name()),
            attachments: AttachmentRegistry::new(),
        };
        control.save().expect("control plane should save");

        let summary =
            sync_control_plane(&control, "personal-client", false).expect("sync should succeed");

        assert!(
            root.join("managed")
                .join("control-plane-export.yaml")
                .exists()
        );
        assert!(summary.tool_paths[0].exists());
        assert!(summary.memory_path.expect("memory path").exists());
    }

    #[test]
    fn can_import_legacy_manifest_into_config() {
        let manifest = r#"
version: 1
workspace: starter-workspace
profiles:
  personal-client:
    memory:
      provider: filesystem
      endpoint: ./.openagents/memory/starter-workspace
      scope: client
    tools:
      codex:
        enabled: true
        guidance_packs: [shared-memory]
"#;

        let parsed = WorkspaceManifest::from_yaml_str(manifest).expect("manifest should parse");
        let config = OpenAgentsConfig::from_manifest(parsed);

        assert_eq!(config.workspace_name, "starter-workspace");
        assert!(config.profiles.contains_key("personal-client"));
    }
}
