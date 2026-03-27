use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
        self.resolve_profile_inner(profile_name, &mut visited)
    }

    fn resolve_profile_inner(
        &self,
        profile_name: &str,
        visited: &mut BTreeSet<String>,
    ) -> Result<ResolvedProfile, ManifestError> {
        if !visited.insert(profile_name.to_string()) {
            return Err(ManifestError::ProfileCycle(profile_name.to_string()));
        }

        let profile = self
            .profiles
            .get(profile_name)
            .ok_or_else(|| ManifestError::UnknownProfile(profile_name.to_string()))?;

        let mut resolved = if let Some(parent_name) = &profile.extends {
            self.resolve_profile_inner(parent_name, visited)?
        } else {
            ResolvedProfile {
                name: profile_name.to_string(),
                description: profile.description.clone(),
                memory: profile.memory.clone(),
                tools: profile.tools.clone(),
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

        Ok(resolved)
    }
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProfile {
    pub name: String,
    pub description: Option<String>,
    pub memory: MemoryConfig,
    pub tools: BTreeMap<ToolKind, ToolConfig>,
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

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to parse manifest: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("manifest version {0} is invalid")]
    InvalidVersion(u32),
    #[error("manifest must define at least one profile")]
    EmptyProfiles,
    #[error("profile `{0}` is not defined")]
    UnknownProfile(String),
    #[error("profile `{0}` extends itself recursively")]
    ProfileCycle(String),
}

#[cfg(test)]
mod tests {
    use crate::{ProfileScope, ToolKind, WorkspaceManifest};

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
    fn errors_for_unknown_profile() {
        let fixture = include_str!("../../../examples/basic/workspace.yaml");
        let manifest = WorkspaceManifest::from_yaml_str(fixture).expect("manifest should parse");

        let error = manifest
            .resolve_profile("missing")
            .expect_err("profile should not resolve");

        assert!(error.to_string().contains("missing"));
    }
}
