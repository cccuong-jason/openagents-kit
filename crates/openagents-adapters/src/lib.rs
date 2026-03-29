use std::path::Path;

use openagents_core::{ResolvedProfile, ToolKind};

pub fn render_adapter_output(
    tool: ToolKind,
    workspace_name: &str,
    profile: &ResolvedProfile,
) -> Result<String, AdapterError> {
    let mut guidance_packs = profile
        .tools
        .values()
        .flat_map(|config| config.guidance_packs.iter().cloned())
        .collect::<Vec<_>>();
    guidance_packs.sort();
    guidance_packs.dedup();
    let guidance = if guidance_packs.is_empty() {
        "none".to_string()
    } else {
        guidance_packs.join(", ")
    };

    let skills = if profile.skills.is_empty() {
        "none".to_string()
    } else {
        profile.skills.join(", ")
    };

    let mcps = if profile.mcp_servers.is_empty() {
        "none".to_string()
    } else {
        profile.mcp_servers.join(", ")
    };

    let rendered = match tool {
        ToolKind::Codex => format!(
            "# Managed by OpenAgents Kit\n# Workspace: {workspace_name}\n# Profile: {}\n# Memory provider: {}\n# Memory endpoint: {}\n# Guidance packs: {guidance}\n# Skills: {skills}\n# MCP servers: {mcps}\n# Managed catalog assets live under the OpenAgents config root.\n# Review this snippet before merging it into ~/.codex/config.toml.\n\n[projects.'<workspace-root>']\ntrust_level = \"trusted\"\n\n# OpenAgents-managed capability inventory\n# skills = [{skills}]\n# mcp_servers = [{mcps}]\n",
            profile.name, profile.memory.provider, profile.memory.endpoint
        ),
        ToolKind::Claude => format!(
            "# OpenAgents Kit\n\nProfile: `{}`\nWorkspace: `{workspace_name}`\nMemory provider: `{}`\nMemory endpoint: `{}`\nGuidance packs: `{guidance}`\nSkills: `{skills}`\nMCP servers: `{mcps}`\nManaged next step: run `openagents-kit sync --profile {}` after updating your OpenAgents control plane.\n",
            profile.name, profile.memory.provider, profile.memory.endpoint, profile.name
        ),
        ToolKind::Gemini => format!(
            "# GEMINI profile bootstrap\n\nprofile: {}\nworkspace: {workspace_name}\nmemory_provider: {}\nmemory_endpoint: {}\nguidance: {guidance}\nskills: {skills}\nmcp_servers: {mcps}\nmanaged_sync: openagents-kit sync --profile {}\n",
            profile.name, profile.memory.provider, profile.memory.endpoint, profile.name
        ),
    };

    Ok(rendered)
}

pub fn write_adapter_output(
    root: &Path,
    tool: ToolKind,
    rendered: &str,
) -> Result<(), AdapterError> {
    let tool_dir = root.join(tool.to_string());
    std::fs::create_dir_all(&tool_dir)?;
    std::fs::write(tool_dir.join(tool.file_name()), rendered)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use crate::render_adapter_output;
    use openagents_core::{OpenAgentsConfig, ToolKind};

    #[test]
    fn renders_codex_output_with_skills_and_mcps() {
        let fixture = r#"
schema: openagents/v1
version: 1
workspace_name: control-plane
default_profile: personal-client
profiles:
  personal-client:
    description: Personal client profile.
    memory:
      provider: filesystem
      endpoint: memory/personal-client
      scope: client
    skills: [shared-memory]
    mcp_servers: [filesystem-memory]
    tools:
      codex:
        enabled: true
        guidance_packs: [shared-memory]
"#;

        let config = OpenAgentsConfig::from_yaml_str(fixture).expect("config should parse");
        let profile = config
            .resolve_profile("personal-client")
            .expect("profile should resolve");

        let rendered = render_adapter_output(ToolKind::Codex, &config.workspace_name, &profile)
            .expect("codex output should render");

        assert!(rendered.contains("Managed by OpenAgents Kit"));
        assert!(rendered.contains("shared-memory"));
        assert!(rendered.contains("filesystem-memory"));
    }

    #[test]
    fn renders_claude_output_with_profile_context() {
        let fixture = r#"
schema: openagents/v1
version: 1
workspace_name: control-plane
default_profile: team-workspace
profiles:
  team-workspace:
    description: Team workspace profile.
    memory:
      provider: filesystem
      endpoint: memory/team-workspace
      scope: team
    skills: [team-handoff]
    mcp_servers: [context7]
    tools:
      claude:
        enabled: true
        guidance_packs: [team-handoff]
"#;

        let config = OpenAgentsConfig::from_yaml_str(fixture).expect("config should parse");
        let profile = config
            .resolve_profile("team-workspace")
            .expect("profile should resolve");

        let rendered = render_adapter_output(ToolKind::Claude, &config.workspace_name, &profile)
            .expect("claude output should render");

        assert!(rendered.contains("team-workspace"));
        assert!(rendered.contains("context7"));
        assert!(rendered.contains("openagents-kit sync --profile team-workspace"));
    }
}
