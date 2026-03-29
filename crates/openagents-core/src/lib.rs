use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const OPENAGENTS_SCHEMA: &str = "openagents/v1";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkspaceManifest {
    pub version: u32,
    pub workspace: String,
    pub profiles: BTreeMap<String, Profile>,
}

impl WorkspaceManifest {
    pub fn from_yaml_str(input: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_yaml::from_str(input)?;
        if manifest.version == 0 {
            return Err(ManifestError::InvalidVersion(manifest.version));
        }
        if manifest.profiles.is_empty() {
            return Err(ManifestError::EmptyProfiles);
        }
        Ok(manifest)
    }

    pub fn resolve_profile(&self, profile_name: &str) -> Result<ResolvedProfile, ManifestError> {
        let mut visited = BTreeSet::new();
        resolve_profile_map(&self.profiles, profile_name, &mut visited)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAgentsConfig {
    pub schema: String,
    pub version: u32,
    pub workspace_name: String,
    pub default_profile: String,
    pub profiles: BTreeMap<String, Profile>,
    #[serde(default)]
    pub custom_catalog: BTreeMap<String, CustomCatalogItem>,
}

impl OpenAgentsConfig {
    pub fn new(workspace_name: impl Into<String>, default_profile: impl Into<String>) -> Self {
        Self {
            schema: OPENAGENTS_SCHEMA.to_string(),
            version: 1,
            workspace_name: workspace_name.into(),
            default_profile: default_profile.into(),
            profiles: BTreeMap::new(),
            custom_catalog: BTreeMap::new(),
        }
    }

    pub fn from_yaml_str(input: &str) -> Result<Self, ManifestError> {
        let config: Self = serde_yaml::from_str(input)?;
        if config.schema != OPENAGENTS_SCHEMA {
            return Err(ManifestError::UnknownSchema(config.schema));
        }
        if config.version == 0 {
            return Err(ManifestError::InvalidVersion(config.version));
        }
        if config.profiles.is_empty() {
            return Err(ManifestError::EmptyProfiles);
        }
        if !config.profiles.contains_key(&config.default_profile) {
            return Err(ManifestError::UnknownProfile(
                config.default_profile.clone(),
            ));
        }
        Ok(config)
    }

    pub fn from_manifest(manifest: WorkspaceManifest) -> Self {
        let default_profile = manifest
            .profiles
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "personal-client".to_string());

        Self {
            schema: OPENAGENTS_SCHEMA.to_string(),
            version: manifest.version,
            workspace_name: manifest.workspace,
            default_profile,
            profiles: manifest.profiles,
            custom_catalog: BTreeMap::new(),
        }
    }

    pub fn resolve_profile(&self, profile_name: &str) -> Result<ResolvedProfile, ManifestError> {
        let mut visited = BTreeSet::new();
        resolve_profile_map(&self.profiles, profile_name, &mut visited)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceOverlay {
    pub schema: String,
    pub version: u32,
    pub device_name: String,
    #[serde(default)]
    pub managed_root: Option<String>,
    #[serde(default)]
    pub memory_root: Option<String>,
}

impl DeviceOverlay {
    pub fn new(device_name: impl Into<String>) -> Self {
        Self {
            schema: OPENAGENTS_SCHEMA.to_string(),
            version: 1,
            device_name: device_name.into(),
            managed_root: None,
            memory_root: None,
        }
    }

    pub fn from_yaml_str(input: &str) -> Result<Self, ManifestError> {
        let overlay: Self = serde_yaml::from_str(input)?;
        if overlay.schema != OPENAGENTS_SCHEMA {
            return Err(ManifestError::UnknownSchema(overlay.schema));
        }
        if overlay.version == 0 {
            return Err(ManifestError::InvalidVersion(overlay.version));
        }
        Ok(overlay)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AttachmentRegistry {
    pub schema: String,
    pub version: u32,
    #[serde(default)]
    pub attachments: Vec<ProjectAttachment>,
}

impl AttachmentRegistry {
    pub fn new() -> Self {
        Self {
            schema: OPENAGENTS_SCHEMA.to_string(),
            version: 1,
            attachments: Vec::new(),
        }
    }

    pub fn from_yaml_str(input: &str) -> Result<Self, ManifestError> {
        let registry: Self = serde_yaml::from_str(input)?;
        if registry.schema != OPENAGENTS_SCHEMA {
            return Err(ManifestError::UnknownSchema(registry.schema));
        }
        if registry.version == 0 {
            return Err(ManifestError::InvalidVersion(registry.version));
        }
        Ok(registry)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectAttachment {
    pub path: String,
    pub profile: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomCatalogItem {
    pub kind: CatalogItemKind,
    pub description: String,
    #[serde(default)]
    pub supported_tools: Vec<ToolKind>,
    pub install_summary: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogItemKind {
    Skill,
    Mcp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    pub memory: MemoryConfig,
    #[serde(default)]
    pub tools: BTreeMap<ToolKind, ToolConfig>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProfile {
    pub name: String,
    pub description: Option<String>,
    pub memory: MemoryConfig,
    pub tools: BTreeMap<ToolKind, ToolConfig>,
    pub skills: Vec<String>,
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct MemoryConfig {
    pub provider: String,
    pub endpoint: String,
    pub scope: ProfileScope,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileScope {
    Client,
    Team,
    Project,
    Workspace,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct ToolConfig {
    pub enabled: bool,
    #[serde(default)]
    pub guidance_packs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    Codex,
    Claude,
    Gemini,
}

impl ToolKind {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Codex => "config.toml",
            Self::Claude => "CLAUDE.md",
            Self::Gemini => "GEMINI.md",
        }
    }
}

impl std::fmt::Display for ToolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codex => write!(f, "codex"),
            Self::Claude => write!(f, "claude"),
            Self::Gemini => write!(f, "gemini"),
        }
    }
}

fn resolve_profile_map(
    profiles: &BTreeMap<String, Profile>,
    profile_name: &str,
    visited: &mut BTreeSet<String>,
) -> Result<ResolvedProfile, ManifestError> {
    if !visited.insert(profile_name.to_string()) {
        return Err(ManifestError::ProfileCycle(profile_name.to_string()));
    }

    let profile = profiles
        .get(profile_name)
        .ok_or_else(|| ManifestError::UnknownProfile(profile_name.to_string()))?;

    let mut resolved = if let Some(parent_name) = &profile.extends {
        resolve_profile_map(profiles, parent_name, visited)?
    } else {
        ResolvedProfile {
            name: profile_name.to_string(),
            description: profile.description.clone(),
            memory: profile.memory.clone(),
            tools: profile.tools.clone(),
            skills: profile.skills.clone(),
            mcp_servers: profile.mcp_servers.clone(),
        }
    };

    resolved.name = profile_name.to_string();
    resolved.description = profile
        .description
        .clone()
        .or_else(|| resolved.description.clone());
    resolved.memory = profile.memory.clone();

    for (tool, config) in &profile.tools {
        resolved.tools.insert(*tool, config.clone());
    }

    merge_unique(&mut resolved.skills, &profile.skills);
    merge_unique(&mut resolved.mcp_servers, &profile.mcp_servers);

    Ok(resolved)
}

fn merge_unique(target: &mut Vec<String>, source: &[String]) {
    for item in source {
        if !target.contains(item) {
            target.push(item.clone());
        }
    }
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to parse yaml: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("schema `{0}` is not supported")]
    UnknownSchema(String),
    #[error("version {0} is invalid")]
    InvalidVersion(u32),
    #[error("config must define at least one profile")]
    EmptyProfiles,
    #[error("profile `{0}` is not defined")]
    UnknownProfile(String),
    #[error("profile `{0}` extends itself recursively")]
    ProfileCycle(String),
}

#[cfg(test)]
mod tests {
    use crate::{
        AttachmentRegistry, DeviceOverlay, OpenAgentsConfig, ProfileScope, ToolKind,
        WorkspaceManifest,
    };

    #[test]
    fn parses_manifest_and_lists_profiles() {
        let fixture = include_str!("../../../examples/basic/workspace.yaml");

        let manifest = WorkspaceManifest::from_yaml_str(fixture).expect("manifest should parse");

        assert_eq!(manifest.workspace, "openagents-kit");
        assert!(manifest.profiles.contains_key("personal-client"));
        assert!(manifest.profiles.contains_key("company-team"));
    }

    #[test]
    fn resolves_parent_profile_overrides() {
        let fixture = include_str!("../../../examples/basic/workspace.yaml");
        let manifest = WorkspaceManifest::from_yaml_str(fixture).expect("manifest should parse");

        let resolved = manifest
            .resolve_profile("company-team")
            .expect("profile should resolve");

        assert_eq!(resolved.memory.endpoint, "https://company.example.com");
        assert_eq!(resolved.memory.scope, ProfileScope::Team);
        assert_eq!(
            resolved.tools[&ToolKind::Codex].guidance_packs,
            vec!["shared-memory".to_string(), "team-handoff".to_string()]
        );
        assert!(resolved.tools[&ToolKind::Claude].enabled);
    }

    #[test]
    fn imports_manifest_into_global_config() {
        let fixture = include_str!("../../../examples/basic/workspace.yaml");
        let manifest = WorkspaceManifest::from_yaml_str(fixture).expect("manifest should parse");

        let config = OpenAgentsConfig::from_manifest(manifest);

        assert_eq!(config.schema, "openagents/v1");
        assert_eq!(config.workspace_name, "openagents-kit");
        assert_eq!(config.default_profile, "company-team");
    }

    #[test]
    fn resolves_skills_and_mcps_across_profile_inheritance() {
        let yaml = r#"
schema: openagents/v1
version: 1
workspace_name: control-plane
default_profile: parent
profiles:
  parent:
    memory:
      provider: filesystem
      endpoint: memory/parent
      scope: team
    skills: [shared-memory]
    mcp_servers: [filesystem-memory]
    tools: {}
  child:
    extends: parent
    memory:
      provider: filesystem
      endpoint: memory/child
      scope: client
    skills: [starter-guidance]
    mcp_servers: [context7]
    tools: {}
"#;

        let config = OpenAgentsConfig::from_yaml_str(yaml).expect("config should parse");
        let resolved = config
            .resolve_profile("child")
            .expect("child should resolve");

        assert_eq!(resolved.skills, vec!["shared-memory", "starter-guidance"]);
        assert_eq!(resolved.mcp_servers, vec!["filesystem-memory", "context7"]);
        assert_eq!(resolved.memory.scope, ProfileScope::Client);
    }

    #[test]
    fn parses_device_overlay_and_attachments() {
        let overlay = DeviceOverlay::from_yaml_str(
            r#"
schema: openagents/v1
version: 1
device_name: work-laptop
managed_root: managed
memory_root: memory
"#,
        )
        .expect("overlay should parse");

        assert_eq!(overlay.device_name, "work-laptop");

        let attachments = AttachmentRegistry::from_yaml_str(
            r#"
schema: openagents/v1
version: 1
attachments:
  - path: /projects/client-a
    profile: personal-client
"#,
        )
        .expect("attachments should parse");

        assert_eq!(attachments.attachments.len(), 1);
        assert_eq!(attachments.attachments[0].profile, "personal-client");
    }
}
