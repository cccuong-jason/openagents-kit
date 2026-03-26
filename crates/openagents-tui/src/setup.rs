use std::path::Path;

use anyhow::Result;
use openagents_core::{
    MemoryConfig, Profile, ProfileScope, ToolConfig, ToolKind, WorkspaceManifest,
};

use crate::detection::ToolDetection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryBackendPreset {
    Filesystem,
    Cortex,
}

impl MemoryBackendPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Filesystem => "Filesystem",
            Self::Cortex => "Cortex",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Filesystem => Self::Cortex,
            Self::Cortex => Self::Filesystem,
        }
    }

    pub fn previous(self) -> Self {
        self.next()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilePreset {
    PersonalClient,
    TeamWorkspace,
    ProjectSandbox,
}

impl ProfilePreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::PersonalClient => "Personal Client",
            Self::TeamWorkspace => "Team Workspace",
            Self::ProjectSandbox => "Project Sandbox",
        }
    }

    pub fn profile_name(self) -> &'static str {
        match self {
            Self::PersonalClient => "personal-client",
            Self::TeamWorkspace => "team-workspace",
            Self::ProjectSandbox => "project-sandbox",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::PersonalClient => Self::TeamWorkspace,
            Self::TeamWorkspace => Self::ProjectSandbox,
            Self::ProjectSandbox => Self::PersonalClient,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::PersonalClient => Self::ProjectSandbox,
            Self::TeamWorkspace => Self::PersonalClient,
            Self::ProjectSandbox => Self::TeamWorkspace,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupSelection {
    pub workspace_name: String,
    pub profile_preset: ProfilePreset,
    pub memory_backend: MemoryBackendPreset,
    pub enabled_tools: Vec<ToolKind>,
    pub warnings: Vec<String>,
}

pub fn recommended_selection(cwd: &Path, detections: &[ToolDetection]) -> SetupSelection {
    let workspace_name = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("openagents-workspace")
        .to_string();

    let mut enabled_tools = detections.iter().map(|item| item.tool).collect::<Vec<_>>();
    enabled_tools.sort();
    enabled_tools.dedup();

    let warnings = if detections.is_empty() {
        vec!["No supported tool configs were detected, so OpenAgents prepared a guided starter setup.".to_string()]
    } else {
        vec![
            "Review the generated memory endpoint before sharing this setup with a client."
                .to_string(),
        ]
    };

    SetupSelection {
        workspace_name,
        profile_preset: ProfilePreset::PersonalClient,
        memory_backend: MemoryBackendPreset::Filesystem,
        enabled_tools,
        warnings,
    }
}

pub fn selection_to_manifest(selection: &SetupSelection) -> WorkspaceManifest {
    let (description, scope) = match selection.profile_preset {
        ProfilePreset::PersonalClient => ("Personal client profile.", ProfileScope::Client),
        ProfilePreset::TeamWorkspace => ("Team workspace profile.", ProfileScope::Team),
        ProfilePreset::ProjectSandbox => ("Project sandbox profile.", ProfileScope::Project),
    };

    let (provider, endpoint) = match selection.memory_backend {
        MemoryBackendPreset::Filesystem => ("filesystem", "./.openagents/memory"),
        MemoryBackendPreset::Cortex => ("cortex", "https://memory.example.com"),
    };

    let mut tools = std::collections::BTreeMap::new();
    for tool in &selection.enabled_tools {
        let guidance_packs = match tool {
            ToolKind::Codex => vec!["shared-memory".to_string(), "detection-import".to_string()],
            ToolKind::Claude => vec!["shared-memory".to_string(), "starter-guidance".to_string()],
            ToolKind::Gemini => vec!["shared-memory".to_string(), "starter-guidance".to_string()],
        };

        tools.insert(
            *tool,
            ToolConfig {
                enabled: true,
                guidance_packs,
            },
        );
    }

    let mut profiles = std::collections::BTreeMap::new();
    profiles.insert(
        selection.profile_preset.profile_name().to_string(),
        Profile {
            description: Some(description.to_string()),
            extends: None,
            memory: MemoryConfig {
                provider: provider.to_string(),
                endpoint: endpoint.to_string(),
                scope,
            },
            tools,
        },
    );

    WorkspaceManifest {
        version: 1,
        workspace: selection.workspace_name.clone(),
        profiles,
    }
}

pub fn write_manifest(path: &Path, selection: &SetupSelection) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let manifest = selection_to_manifest(selection);
    let serialized = serde_yaml::to_string(&manifest)?;
    std::fs::write(path, serialized)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        MemoryBackendPreset, ProfilePreset, recommended_selection, selection_to_manifest,
        write_manifest,
    };
    use crate::detection::ToolDetection;
    use openagents_core::{ProfileScope, ToolKind, WorkspaceManifest};

    #[test]
    fn builds_detected_selection_with_filesystem_memory_default() {
        let detections = vec![
            ToolDetection {
                tool: ToolKind::Codex,
                evidence_path: "C:/Users/example/.codex/config.toml".into(),
                summary: "Codex config found".to_string(),
            },
            ToolDetection {
                tool: ToolKind::Gemini,
                evidence_path: "C:/Users/example/.gemini/settings.json".into(),
                summary: "Gemini settings found".to_string(),
            },
        ];

        let selection =
            recommended_selection(Path::new("D:/Projects/client-workspace"), &detections);

        assert_eq!(selection.workspace_name, "client-workspace");
        assert_eq!(selection.profile_preset, ProfilePreset::PersonalClient);
        assert_eq!(selection.memory_backend, MemoryBackendPreset::Filesystem);
        assert_eq!(
            selection.enabled_tools,
            vec![ToolKind::Codex, ToolKind::Gemini]
        );
    }

    #[test]
    fn converts_selection_into_manifest() {
        let selection = super::SetupSelection {
            workspace_name: "starter-workspace".to_string(),
            profile_preset: ProfilePreset::TeamWorkspace,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Claude, ToolKind::Gemini],
            warnings: vec!["Detected settings could not be mapped exactly.".to_string()],
        };

        let manifest = selection_to_manifest(&selection);
        let profile = manifest
            .profiles
            .get("team-workspace")
            .expect("team profile should exist");

        assert_eq!(manifest.workspace, "starter-workspace");
        assert_eq!(profile.memory.provider, "filesystem");
        assert_eq!(profile.memory.scope, ProfileScope::Team);
        assert!(profile.tools.contains_key(&ToolKind::Claude));
        assert!(profile.tools.contains_key(&ToolKind::Gemini));
    }

    #[test]
    fn writes_serialized_manifest_to_disk() {
        let temp = tempdir().expect("temp dir should exist");
        let manifest_path = temp.path().join("workspace.yaml");
        let selection = super::SetupSelection {
            workspace_name: "starter".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Codex],
            warnings: Vec::new(),
        };

        write_manifest(&manifest_path, &selection).expect("manifest write should succeed");

        let written = fs::read_to_string(&manifest_path).expect("manifest should be written");
        let manifest = WorkspaceManifest::from_yaml_str(&written).expect("manifest should parse");

        assert_eq!(manifest.workspace, "starter");
        assert!(
            manifest.profiles["personal-client"]
                .tools
                .contains_key(&ToolKind::Codex)
        );
    }
}
