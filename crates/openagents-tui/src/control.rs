use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use openagents_core::{
    AttachmentRegistry, DeviceOverlay, OpenAgentsConfig, ProjectAttachment, ResolvedProfile,
    WorkspaceManifest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneOrigin {
    GlobalConfig,
    LegacyManifest,
}

#[derive(Debug, Clone)]
pub struct ControlPlane {
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub overlay_path: PathBuf,
    pub attachments_path: PathBuf,
    pub origin: ControlPlaneOrigin,
    pub config: OpenAgentsConfig,
    pub overlay: DeviceOverlay,
    pub attachments: AttachmentRegistry,
}

impl ControlPlane {
    pub fn load(config_override: Option<&Path>, manifest_override: Option<&Path>) -> Result<Self> {
        if let Some(manifest_path) = manifest_override {
            return Self::load_from_manifest(manifest_path);
        }

        let root = config_override
            .map(|path| {
                if path.extension().is_some() {
                    path.parent().unwrap_or(Path::new(".")).to_path_buf()
                } else {
                    path.to_path_buf()
                }
            })
            .unwrap_or(default_config_root()?);

        let config_path = if config_override.is_some_and(|path| path.extension().is_some()) {
            config_override.unwrap().to_path_buf()
        } else {
            root.join("config.yaml")
        };

        let overlay_path = root.join("device.yaml");
        let attachments_path = root.join("attachments.yaml");

        let config = OpenAgentsConfig::from_yaml_str(
            &fs::read_to_string(&config_path)
                .with_context(|| format!("failed to read config at {}", config_path.display()))?,
        )
        .context("failed to parse OpenAgents config")?;

        let overlay =
            if overlay_path.exists() {
                DeviceOverlay::from_yaml_str(&fs::read_to_string(&overlay_path).with_context(
                    || format!("failed to read overlay at {}", overlay_path.display()),
                )?)
                .context("failed to parse device overlay")?
            } else {
                DeviceOverlay::new(device_name())
            };

        let attachments = if attachments_path.exists() {
            AttachmentRegistry::from_yaml_str(&fs::read_to_string(&attachments_path).with_context(
                || {
                    format!(
                        "failed to read attachments at {}",
                        attachments_path.display()
                    )
                },
            )?)
            .context("failed to parse attachment registry")?
        } else {
            AttachmentRegistry::new()
        };

        Ok(Self {
            root,
            config_path,
            overlay_path,
            attachments_path,
            origin: ControlPlaneOrigin::GlobalConfig,
            config,
            overlay,
            attachments,
        })
    }

    fn load_from_manifest(manifest_path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(manifest_path)
            .with_context(|| format!("failed to read manifest at {}", manifest_path.display()))?;
        let manifest =
            WorkspaceManifest::from_yaml_str(&contents).context("failed to parse manifest")?;
        let root = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(".openagents-legacy");
        let overlay = DeviceOverlay::new(device_name());
        let attachments = AttachmentRegistry::new();

        Ok(Self {
            config_path: manifest_path.to_path_buf(),
            overlay_path: root.join("device.yaml"),
            attachments_path: root.join("attachments.yaml"),
            root,
            origin: ControlPlaneOrigin::LegacyManifest,
            config: OpenAgentsConfig::from_manifest(manifest),
            overlay,
            attachments,
        })
    }

    pub fn save(&self) -> Result<()> {
        if self.origin == ControlPlaneOrigin::LegacyManifest {
            return Err(anyhow!(
                "cannot save a control plane loaded from a legacy manifest; run setup without --manifest"
            ));
        }

        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        fs::write(&self.config_path, serde_yaml::to_string(&self.config)?)
            .with_context(|| format!("failed to write {}", self.config_path.display()))?;
        fs::write(&self.overlay_path, serde_yaml::to_string(&self.overlay)?)
            .with_context(|| format!("failed to write {}", self.overlay_path.display()))?;
        fs::write(
            &self.attachments_path,
            serde_yaml::to_string(&self.attachments)?,
        )
        .with_context(|| format!("failed to write {}", self.attachments_path.display()))?;
        Ok(())
    }

    pub fn managed_root(&self) -> PathBuf {
        resolve_overlay_path(&self.root, self.overlay.managed_root.as_deref(), "managed")
    }

    pub fn memory_root(&self) -> PathBuf {
        resolve_overlay_path(&self.root, self.overlay.memory_root.as_deref(), "memory")
    }

    pub fn resolved_profile(&self, profile: &str) -> Result<ResolvedProfile> {
        let mut resolved = self
            .config
            .resolve_profile(profile)
            .map_err(|error| anyhow!(error))?;

        if resolved.memory.provider == "filesystem" {
            let endpoint = PathBuf::from(&resolved.memory.endpoint);
            let absolute = if endpoint.is_absolute() {
                endpoint
            } else {
                self.memory_root().join(endpoint)
            };
            resolved.memory.endpoint = absolute.display().to_string();
        }

        Ok(resolved)
    }

    pub fn active_profile_name(&self, cwd: &Path, explicit_profile: Option<&str>) -> String {
        explicit_profile
            .map(str::to_string)
            .or_else(|| self.attached_profile_for(cwd))
            .unwrap_or_else(|| self.config.default_profile.clone())
    }

    pub fn attach_current_path(&mut self, cwd: &Path, profile: &str) {
        let normalized = normalize_path(cwd);
        self.attachments
            .attachments
            .retain(|item| item.path != normalized);
        self.attachments.attachments.push(ProjectAttachment {
            path: normalized,
            profile: profile.to_string(),
        });
        self.attachments
            .attachments
            .sort_by(|left, right| left.path.cmp(&right.path));
    }

    pub fn attached_profile_for(&self, cwd: &Path) -> Option<String> {
        let current = normalize_path(cwd);
        self.attachments
            .attachments
            .iter()
            .filter_map(|attachment| {
                let path = normalize_attachment_path(&attachment.path);
                current
                    .starts_with(&path)
                    .then_some((path.len(), attachment.profile.clone()))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, profile)| profile)
    }
}

pub fn default_config_root() -> Result<PathBuf> {
    if cfg!(windows) {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return Ok(PathBuf::from(appdata).join("OpenAgents"));
        }
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .context("could not determine home directory")?;
    Ok(home.join(".config").join("openagents"))
}

pub fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "current-device".to_string())
}

fn resolve_overlay_path(root: &Path, value: Option<&str>, fallback: &str) -> PathBuf {
    match value {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        }
        None => root.join(fallback),
    }
}

fn normalize_path(path: &Path) -> String {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized = absolute.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn normalize_attachment_path(path: &str) -> String {
    let path_buf = PathBuf::from(path);
    let normalized = path_buf
        .canonicalize()
        .unwrap_or(path_buf)
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use openagents_core::{MemoryConfig, OpenAgentsConfig, Profile, ProfileScope};
    use tempfile::tempdir;

    use super::{AttachmentRegistry, ControlPlane, ControlPlaneOrigin, DeviceOverlay};

    #[test]
    fn attachment_lookup_prefers_longest_prefix() {
        let temp = tempdir().expect("temp dir should exist");
        let root = temp.path().join("config");
        fs::create_dir_all(&root).expect("config root should exist");
        let project_root = temp.path().join("projects");
        let nested = project_root.join("client-a");
        fs::create_dir_all(&nested).expect("project root should exist");

        let mut config = OpenAgentsConfig::new("openagents-home", "personal-client");
        config.profiles.insert(
            "personal-client".to_string(),
            Profile {
                description: None,
                extends: None,
                memory: MemoryConfig {
                    provider: "filesystem".to_string(),
                    endpoint: "memory/personal-client".to_string(),
                    scope: ProfileScope::Client,
                },
                tools: Default::default(),
                skills: vec![],
                mcp_servers: vec![],
            },
        );
        config.profiles.insert(
            "team-workspace".to_string(),
            Profile {
                description: None,
                extends: None,
                memory: MemoryConfig {
                    provider: "filesystem".to_string(),
                    endpoint: "memory/team-workspace".to_string(),
                    scope: ProfileScope::Team,
                },
                tools: Default::default(),
                skills: vec![],
                mcp_servers: vec![],
            },
        );

        let plane = ControlPlane {
            root: root.clone(),
            config_path: root.join("config.yaml"),
            overlay_path: root.join("device.yaml"),
            attachments_path: root.join("attachments.yaml"),
            origin: ControlPlaneOrigin::GlobalConfig,
            config,
            overlay: DeviceOverlay::new("test-box"),
            attachments: AttachmentRegistry {
                schema: "openagents/v1".to_string(),
                version: 1,
                attachments: vec![
                    openagents_core::ProjectAttachment {
                        path: project_root.to_string_lossy().replace('\\', "/"),
                        profile: "team-workspace".to_string(),
                    },
                    openagents_core::ProjectAttachment {
                        path: nested.to_string_lossy().replace('\\', "/"),
                        profile: "personal-client".to_string(),
                    },
                ],
            },
        };

        assert_eq!(
            plane.attached_profile_for(&nested),
            Some("personal-client".to_string())
        );
    }

    #[test]
    fn resolves_filesystem_memory_under_overlay_root() {
        let temp = tempdir().expect("temp dir should exist");
        let root = temp.path().join("config");
        fs::create_dir_all(&root).expect("config root should exist");

        let mut config = OpenAgentsConfig::new("openagents-home", "personal-client");
        config.profiles.insert(
            "personal-client".to_string(),
            Profile {
                description: None,
                extends: None,
                memory: MemoryConfig {
                    provider: "filesystem".to_string(),
                    endpoint: "profiles/personal-client".to_string(),
                    scope: ProfileScope::Client,
                },
                tools: Default::default(),
                skills: vec![],
                mcp_servers: vec![],
            },
        );

        let plane = ControlPlane {
            root: root.clone(),
            config_path: root.join("config.yaml"),
            overlay_path: root.join("device.yaml"),
            attachments_path: root.join("attachments.yaml"),
            origin: ControlPlaneOrigin::GlobalConfig,
            config,
            overlay: DeviceOverlay {
                schema: "openagents/v1".to_string(),
                version: 1,
                device_name: "test-box".to_string(),
                managed_root: None,
                memory_root: Some("state/memory".to_string()),
            },
            attachments: AttachmentRegistry::new(),
        };

        let resolved = plane
            .resolved_profile("personal-client")
            .expect("profile should resolve");

        let expected = root
            .join("state")
            .join("memory")
            .join("profiles")
            .join("personal-client");
        assert_eq!(PathBuf::from(resolved.memory.endpoint), expected);
    }
}
