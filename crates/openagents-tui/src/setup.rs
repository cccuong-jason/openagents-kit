use std::collections::BTreeMap;

use openagents_core::{
    MemoryConfig, OpenAgentsConfig, Profile, ProfileScope, ToolConfig, ToolKind,
};

use crate::catalog::{recommended_mcp_ids, recommended_skill_ids};
use crate::detection::DetectionReport;

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

    pub fn provider_name(self) -> &'static str {
        match self {
            Self::Filesystem => "filesystem",
            Self::Cortex => "cortex",
        }
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupSelection {
    pub workspace_name: String,
    pub profile_preset: ProfilePreset,
    pub memory_backend: MemoryBackendPreset,
    pub enabled_tools: Vec<ToolKind>,
    pub selected_skills: Vec<String>,
    pub selected_mcp_servers: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupQuestion {
    Profile,
    Memory,
    Tools,
    Skills,
    Mcps,
    Confirm,
}

pub fn recommended_selection(detections: &DetectionReport) -> SetupSelection {
    let mut enabled_tools = detections
        .detections
        .iter()
        .map(|item| item.tool)
        .collect::<Vec<_>>();
    if enabled_tools.is_empty() {
        enabled_tools = vec![ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini];
    }
    enabled_tools.sort();
    enabled_tools.dedup();

    let mut warnings = if detections.detections.is_empty() {
        vec![
            "I did not find a trusted tool footprint, so I prepared a starter control plane."
                .to_string(),
        ]
    } else {
        vec![
            "I can sync the same desired capabilities across your enabled tools after setup."
                .to_string(),
        ]
    };

    if !detections.has_memory_layer {
        warnings.push(
            "I did not detect an existing memory layer, so I will recommend a local one first."
                .to_string(),
        );
    }

    let profile_preset = ProfilePreset::PersonalClient;
    let memory_backend = MemoryBackendPreset::Filesystem;
    let mut selected_skills = recommended_skill_ids(profile_preset.profile_name());
    let mut selected_mcp_servers = recommended_mcp_ids(
        profile_preset.profile_name(),
        memory_backend.provider_name(),
    );

    merge_unique(&mut selected_skills, &detections.installed_skills);
    merge_unique(&mut selected_mcp_servers, &detections.installed_mcp_servers);

    SetupSelection {
        workspace_name: "openagents-home".to_string(),
        profile_preset,
        memory_backend,
        enabled_tools,
        selected_skills,
        selected_mcp_servers,
        warnings,
    }
}

pub fn refresh_catalog_recommendations(selection: &mut SetupSelection) {
    selection.selected_skills = recommended_skill_ids(selection.profile_preset.profile_name());
    selection.selected_mcp_servers = recommended_mcp_ids(
        selection.profile_preset.profile_name(),
        selection.memory_backend.provider_name(),
    );
}

pub fn selection_to_config(selection: &SetupSelection) -> OpenAgentsConfig {
    let (description, scope) = match selection.profile_preset {
        ProfilePreset::PersonalClient => ("Personal client profile.", ProfileScope::Client),
        ProfilePreset::TeamWorkspace => ("Team workspace profile.", ProfileScope::Team),
        ProfilePreset::ProjectSandbox => ("Project sandbox profile.", ProfileScope::Project),
    };

    let endpoint = match selection.memory_backend {
        MemoryBackendPreset::Filesystem => {
            format!(
                "profiles/{}",
                sanitize_profile_name(selection.profile_preset.profile_name())
            )
        }
        MemoryBackendPreset::Cortex => format!(
            "https://memory.example.com/{}",
            sanitize_profile_name(selection.profile_preset.profile_name())
        ),
    };

    let mut tools = BTreeMap::new();
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

    let profile_name = selection.profile_preset.profile_name().to_string();
    let mut config = OpenAgentsConfig::new(&selection.workspace_name, &profile_name);
    config.profiles.insert(
        profile_name,
        Profile {
            description: Some(description.to_string()),
            extends: None,
            memory: MemoryConfig {
                provider: selection.memory_backend.provider_name().to_string(),
                endpoint,
                scope,
            },
            tools,
            skills: selection.selected_skills.clone(),
            mcp_servers: selection.selected_mcp_servers.clone(),
        },
    );

    config
}

pub fn selection_from_config(config: &OpenAgentsConfig) -> SetupSelection {
    let profile_name = config.default_profile.as_str();
    let profile = config
        .profiles
        .get(profile_name)
        .or_else(|| config.profiles.values().next())
        .expect("config should have at least one profile");

    let profile_preset = match profile_name {
        "team-workspace" => ProfilePreset::TeamWorkspace,
        "project-sandbox" => ProfilePreset::ProjectSandbox,
        _ => ProfilePreset::PersonalClient,
    };

    let memory_backend = match profile.memory.provider.as_str() {
        "cortex" => MemoryBackendPreset::Cortex,
        _ => MemoryBackendPreset::Filesystem,
    };

    let mut enabled_tools = profile
        .tools
        .iter()
        .filter_map(|(tool, config)| config.enabled.then_some(*tool))
        .collect::<Vec<_>>();
    enabled_tools.sort();

    SetupSelection {
        workspace_name: config.workspace_name.clone(),
        profile_preset,
        memory_backend,
        enabled_tools,
        selected_skills: profile.skills.clone(),
        selected_mcp_servers: profile.mcp_servers.clone(),
        warnings: vec!["I imported your existing OpenAgents control plane.".to_string()],
    }
}

fn sanitize_profile_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn merge_unique(target: &mut Vec<String>, source: &[String]) {
    for item in source {
        if !target.contains(item) {
            target.push(item.clone());
        }
    }
    target.sort();
    target.dedup();
}

pub fn setup_questions(
    detections: &DetectionReport,
    selection: &SetupSelection,
    existing_control_plane: bool,
) -> Vec<SetupQuestion> {
    let mut questions = Vec::new();

    if !existing_control_plane {
        questions.push(SetupQuestion::Profile);
    }

    if !existing_control_plane || !detections.has_memory_layer {
        questions.push(SetupQuestion::Memory);
    }

    let detected_tools = detections
        .detections
        .iter()
        .map(|item| item.tool)
        .collect::<Vec<_>>();
    let has_missing_selected_tools = selection
        .enabled_tools
        .iter()
        .any(|tool| !detected_tools.contains(tool));

    if selection.enabled_tools.is_empty()
        || has_missing_selected_tools
        || (!existing_control_plane && detections.detections.is_empty())
    {
        questions.push(SetupQuestion::Tools);
    }

    let missing_skills = selection
        .selected_skills
        .iter()
        .any(|skill| !detections.installed_skills.contains(skill));
    if missing_skills {
        questions.push(SetupQuestion::Skills);
    }

    let missing_mcps = selection
        .selected_mcp_servers
        .iter()
        .any(|server| !detections.installed_mcp_servers.contains(server));
    if missing_mcps {
        questions.push(SetupQuestion::Mcps);
    }

    questions.push(SetupQuestion::Confirm);
    questions
}

#[cfg(test)]
mod tests {
    use crate::detection::{DetectionReport, ToolDetection};
    use openagents_core::{OpenAgentsConfig, ProfileScope, ToolKind};

    use super::{
        MemoryBackendPreset, ProfilePreset, SetupQuestion, recommended_selection,
        refresh_catalog_recommendations, selection_from_config, selection_to_config,
        setup_questions,
    };

    #[test]
    fn builds_detected_selection_with_catalog_defaults() {
        let detections = DetectionReport {
            detections: vec![
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
            ],
            warnings: Vec::new(),
            installed_skills: vec!["team-handoff".to_string()],
            installed_mcp_servers: vec!["context7".to_string()],
            has_memory_layer: false,
        };

        let selection = recommended_selection(&detections);

        assert_eq!(selection.workspace_name, "openagents-home");
        assert_eq!(selection.profile_preset, ProfilePreset::PersonalClient);
        assert_eq!(selection.memory_backend, MemoryBackendPreset::Filesystem);
        assert_eq!(
            selection.enabled_tools,
            vec![ToolKind::Codex, ToolKind::Gemini]
        );
        assert!(
            selection
                .selected_skills
                .contains(&"team-handoff".to_string())
        );
        assert!(
            selection
                .selected_mcp_servers
                .contains(&"filesystem-memory".to_string())
        );
    }

    #[test]
    fn converts_selection_into_global_config() {
        let selection = super::SetupSelection {
            workspace_name: "starter-home".to_string(),
            profile_preset: ProfilePreset::TeamWorkspace,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Claude, ToolKind::Gemini],
            selected_skills: vec!["team-handoff".to_string()],
            selected_mcp_servers: vec!["filesystem-memory".to_string()],
            warnings: vec!["Detected settings could not be mapped exactly.".to_string()],
        };

        let config = selection_to_config(&selection);
        let profile = config
            .profiles
            .get("team-workspace")
            .expect("team profile should exist");

        assert_eq!(config.workspace_name, "starter-home");
        assert_eq!(profile.memory.provider, "filesystem");
        assert_eq!(profile.memory.endpoint, "profiles/team-workspace");
        assert_eq!(profile.memory.scope, ProfileScope::Team);
        assert!(profile.tools.contains_key(&ToolKind::Claude));
        assert!(profile.tools.contains_key(&ToolKind::Gemini));
        assert_eq!(profile.skills, vec!["team-handoff".to_string()]);
    }

    #[test]
    fn refreshes_catalog_items_after_profile_change() {
        let mut selection = super::SetupSelection {
            workspace_name: "starter-home".to_string(),
            profile_preset: ProfilePreset::ProjectSandbox,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Codex],
            selected_skills: vec![],
            selected_mcp_servers: vec![],
            warnings: vec![],
        };

        refresh_catalog_recommendations(&mut selection);

        assert!(
            selection
                .selected_skills
                .contains(&"starter-guidance".to_string())
        );
        assert!(
            selection
                .selected_mcp_servers
                .contains(&"filesystem-memory".to_string())
        );
    }

    #[test]
    fn reconstructs_selection_from_existing_config() {
        let yaml = r#"
schema: openagents/v1
version: 1
workspace_name: openagents-home
default_profile: personal-client
profiles:
  personal-client:
    memory:
      provider: filesystem
      endpoint: profiles/personal-client
      scope: client
    skills: [shared-memory]
    mcp_servers: [filesystem-memory]
    tools:
      codex:
        enabled: true
        guidance_packs: [shared-memory]
"#;

        let config = OpenAgentsConfig::from_yaml_str(yaml).expect("config should parse");
        let selection = selection_from_config(&config);

        assert_eq!(selection.profile_preset, ProfilePreset::PersonalClient);
        assert_eq!(selection.memory_backend, MemoryBackendPreset::Filesystem);
        assert_eq!(selection.enabled_tools, vec![ToolKind::Codex]);
    }

    #[test]
    fn skips_healthy_follow_up_questions_when_existing_setup_is_already_complete() {
        let report = DetectionReport {
            detections: vec![
                ToolDetection {
                    tool: ToolKind::Codex,
                    evidence_path: "C:/Users/example/.codex/config.toml".into(),
                    summary: "Codex config found".to_string(),
                },
                ToolDetection {
                    tool: ToolKind::Claude,
                    evidence_path: "C:/Users/example/.claude.json".into(),
                    summary: "Claude state found".to_string(),
                },
            ],
            warnings: Vec::new(),
            installed_skills: vec!["shared-memory".to_string()],
            installed_mcp_servers: vec!["filesystem-memory".to_string()],
            has_memory_layer: true,
        };
        let selection = super::SetupSelection {
            workspace_name: "openagents-home".to_string(),
            profile_preset: ProfilePreset::PersonalClient,
            memory_backend: MemoryBackendPreset::Filesystem,
            enabled_tools: vec![ToolKind::Claude, ToolKind::Codex],
            selected_skills: vec!["shared-memory".to_string()],
            selected_mcp_servers: vec!["filesystem-memory".to_string()],
            warnings: vec![],
        };

        let questions = setup_questions(&report, &selection, true);

        assert_eq!(questions, vec![SetupQuestion::Confirm]);
    }

    #[test]
    fn first_time_setup_focuses_on_missing_memory_and_catalog_gaps() {
        let report = DetectionReport {
            detections: vec![
                ToolDetection {
                    tool: ToolKind::Codex,
                    evidence_path: "C:/Users/example/.codex/config.toml".into(),
                    summary: "Codex config found".to_string(),
                },
                ToolDetection {
                    tool: ToolKind::Claude,
                    evidence_path: "C:/Users/example/.claude.json".into(),
                    summary: "Claude state found".to_string(),
                },
                ToolDetection {
                    tool: ToolKind::Gemini,
                    evidence_path: "C:/Users/example/.gemini/settings.json".into(),
                    summary: "Gemini settings found".to_string(),
                },
            ],
            warnings: Vec::new(),
            installed_skills: vec!["starter-guidance".to_string()],
            installed_mcp_servers: vec![],
            has_memory_layer: false,
        };
        let selection = recommended_selection(&report);

        let questions = setup_questions(&report, &selection, false);

        assert_eq!(
            questions,
            vec![
                SetupQuestion::Profile,
                SetupQuestion::Memory,
                SetupQuestion::Skills,
                SetupQuestion::Mcps,
                SetupQuestion::Confirm,
            ]
        );
    }
}
