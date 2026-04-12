use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use openagents_core::{
    CatalogItemKind, CatalogItemRecord, CatalogMcpEndpoint, CatalogMcpTransport, ResolvedProfile,
    ToolKind,
};
use serde_json::{Map as JsonMap, Value as JsonValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSyncSummary {
    pub tool: ToolKind,
    pub config_path: PathBuf,
    pub skill_paths: Vec<PathBuf>,
    pub managed_mcp_servers: Vec<String>,
    pub drift_detected: bool,
}

#[derive(Debug, Clone)]
pub struct ToolSyncContext {
    pub workspace_name: String,
    pub profile_name: String,
    pub memory_provider: String,
    pub memory_endpoint: String,
}

pub fn reconcile_tool_configs(
    home: &Path,
    context: &ToolSyncContext,
    profile: &ResolvedProfile,
    catalog: &BTreeMap<String, CatalogItemRecord>,
    dry_run: bool,
) -> Result<Vec<ToolSyncSummary>, crate::AdapterError> {
    let mut summaries = Vec::new();

    for tool in profile.tools.keys().copied() {
        let mcp_servers = materialize_mcp_servers(tool, profile, catalog);
        let skill_bodies = materialize_skill_bodies(tool, profile, catalog);
        let summary = match tool {
            ToolKind::Codex => {
                reconcile_codex_config(home, context, &profile.skills, &mcp_servers, dry_run)?
            }
            ToolKind::Claude => reconcile_json_tool_config(JsonToolSyncTarget {
                tool,
                config_path: home.join(".claude.json"),
                skill_root: home.join(".claude/commands"),
                context,
                skills: &profile.skills,
                skill_bodies: &skill_bodies,
                mcp_servers: &mcp_servers,
                dry_run,
            })?,
            ToolKind::Gemini => reconcile_json_tool_config(JsonToolSyncTarget {
                tool,
                config_path: home.join(".gemini/settings.json"),
                skill_root: home.join(".gemini/extensions"),
                context,
                skills: &profile.skills,
                skill_bodies: &skill_bodies,
                mcp_servers: &mcp_servers,
                dry_run,
            })?,
        };
        summaries.push(summary);
    }

    Ok(summaries)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializedMcpServer {
    id: String,
    endpoint: CatalogMcpEndpoint,
}

struct JsonToolSyncTarget<'a> {
    tool: ToolKind,
    config_path: PathBuf,
    skill_root: PathBuf,
    context: &'a ToolSyncContext,
    skills: &'a [String],
    skill_bodies: &'a [(String, String)],
    mcp_servers: &'a [MaterializedMcpServer],
    dry_run: bool,
}

fn materialize_mcp_servers(
    tool: ToolKind,
    profile: &ResolvedProfile,
    catalog: &BTreeMap<String, CatalogItemRecord>,
) -> Vec<MaterializedMcpServer> {
    profile
        .mcp_servers
        .iter()
        .filter_map(|id| {
            let item = catalog.get(id)?;
            let recipe = item.install.mcp.as_ref()?;
            let endpoint = match tool {
                ToolKind::Codex => recipe.codex.clone(),
                ToolKind::Claude => recipe.claude.clone(),
                ToolKind::Gemini => recipe.gemini.clone(),
            }?;
            Some(MaterializedMcpServer {
                id: id.clone(),
                endpoint,
            })
        })
        .collect()
}

fn materialize_skill_bodies(
    tool: ToolKind,
    profile: &ResolvedProfile,
    catalog: &BTreeMap<String, CatalogItemRecord>,
) -> Vec<(String, String)> {
    if tool == ToolKind::Codex {
        return Vec::new();
    }

    profile
        .skills
        .iter()
        .filter_map(|id| {
            let item = catalog.get(id)?;
            if item.kind != CatalogItemKind::Skill {
                return None;
            }
            let body = item.install.managed_files.first()?.body.clone();
            Some((id.clone(), body))
        })
        .collect()
}

fn reconcile_codex_config(
    home: &Path,
    context: &ToolSyncContext,
    skills: &[String],
    mcp_servers: &[MaterializedMcpServer],
    dry_run: bool,
) -> Result<ToolSyncSummary, crate::AdapterError> {
    let path = home.join(".codex").join("config.toml");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let managed_block = render_codex_managed_block(context, skills, mcp_servers);
    let rendered = upsert_managed_block(
        &existing,
        "# OPENAGENTS BEGIN",
        "# OPENAGENTS END",
        &managed_block,
    );
    let drift_detected = rendered != existing;

    if !dry_run {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, rendered)?;
    }

    Ok(ToolSyncSummary {
        tool: ToolKind::Codex,
        config_path: path,
        skill_paths: Vec::new(),
        managed_mcp_servers: mcp_servers.iter().map(|item| item.id.clone()).collect(),
        drift_detected,
    })
}

fn reconcile_json_tool_config(
    target: JsonToolSyncTarget<'_>,
) -> Result<ToolSyncSummary, crate::AdapterError> {
    let JsonToolSyncTarget {
        tool,
        config_path,
        skill_root,
        context,
        skills,
        skill_bodies,
        mcp_servers,
        dry_run,
    } = target;

    let existing = std::fs::read_to_string(&config_path).unwrap_or_else(|_| "{}".to_string());
    let mut document = serde_json::from_str::<JsonValue>(&existing)
        .unwrap_or_else(|_| JsonValue::Object(JsonMap::new()));
    let mut root = document.as_object().cloned().unwrap_or_default();

    let previous_managed = root
        .get("openAgents")
        .and_then(JsonValue::as_object)
        .and_then(|openagents| openagents.get("managedMcpServers"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();

    let mcp_object = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let mut mcp_map = mcp_object.as_object().cloned().unwrap_or_default();
    for stale in previous_managed {
        if !mcp_servers.iter().any(|item| item.id == stale) {
            mcp_map.remove(&stale);
        }
    }
    for server in mcp_servers {
        mcp_map.insert(server.id.clone(), endpoint_to_json(&server.endpoint));
    }
    root.insert("mcpServers".to_string(), JsonValue::Object(mcp_map));

    let mut openagents = JsonMap::new();
    openagents.insert(
        "workspace".to_string(),
        JsonValue::String(context.workspace_name.clone()),
    );
    openagents.insert(
        "profile".to_string(),
        JsonValue::String(context.profile_name.clone()),
    );
    openagents.insert(
        "memoryProvider".to_string(),
        JsonValue::String(context.memory_provider.clone()),
    );
    openagents.insert(
        "memoryEndpoint".to_string(),
        JsonValue::String(context.memory_endpoint.clone()),
    );
    openagents.insert(
        "skills".to_string(),
        JsonValue::Array(skills.iter().cloned().map(JsonValue::String).collect()),
    );
    openagents.insert(
        "managedMcpServers".to_string(),
        JsonValue::Array(
            mcp_servers
                .iter()
                .map(|item| JsonValue::String(item.id.clone()))
                .collect(),
        ),
    );
    root.insert("openAgents".to_string(), JsonValue::Object(openagents));

    document = JsonValue::Object(root);
    let rendered = serde_json::to_string_pretty(&document)?;
    let drift_detected = normalize_json(&existing) != normalize_json(&rendered);

    let mut skill_paths = Vec::new();
    for (id, body) in skill_bodies {
        let path = skill_root.join(format!("{id}.md"));
        skill_paths.push(path.clone());
        if !dry_run {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, body)?;
        }
    }

    if !dry_run {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&config_path, rendered)?;
    }

    Ok(ToolSyncSummary {
        tool,
        config_path,
        skill_paths,
        managed_mcp_servers: mcp_servers.iter().map(|item| item.id.clone()).collect(),
        drift_detected,
    })
}

fn render_codex_managed_block(
    context: &ToolSyncContext,
    skills: &[String],
    mcp_servers: &[MaterializedMcpServer],
) -> String {
    let mut lines = vec![
        "# OPENAGENTS BEGIN".to_string(),
        format!("# workspace = {}", context.workspace_name),
        format!("# profile = {}", context.profile_name),
        format!("# memory_provider = {}", context.memory_provider),
        format!("# memory_endpoint = {}", context.memory_endpoint),
        format!(
            "# skills = {}",
            if skills.is_empty() {
                "none".to_string()
            } else {
                skills.join(", ")
            }
        ),
    ];

    for server in mcp_servers {
        lines.push(render_codex_mcp_section(server));
    }

    lines.push("# OPENAGENTS END".to_string());
    lines.join("\n")
}

fn render_codex_mcp_section(server: &MaterializedMcpServer) -> String {
    match server.endpoint.transport {
        CatalogMcpTransport::Stdio => {
            let command = server.endpoint.command.clone().unwrap_or_default();
            let args = server
                .endpoint
                .args
                .iter()
                .map(|arg| format!("\"{arg}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "\n[mcp_servers.\"{}\"]\ncommand = \"{}\"\nargs = [{}]",
                server.id, command, args
            )
        }
        CatalogMcpTransport::Http => {
            let url = server.endpoint.url.clone().unwrap_or_default();
            format!("\n[mcp_servers.\"{}\"]\nurl = \"{}\"", server.id, url)
        }
    }
}

fn endpoint_to_json(endpoint: &CatalogMcpEndpoint) -> JsonValue {
    match endpoint.transport {
        CatalogMcpTransport::Stdio => {
            let mut map = JsonMap::new();
            map.insert(
                "command".to_string(),
                JsonValue::String(endpoint.command.clone().unwrap_or_default()),
            );
            map.insert(
                "args".to_string(),
                JsonValue::Array(
                    endpoint
                        .args
                        .iter()
                        .cloned()
                        .map(JsonValue::String)
                        .collect(),
                ),
            );
            if !endpoint.env.is_empty() {
                map.insert(
                    "env".to_string(),
                    JsonValue::Object(
                        endpoint
                            .env
                            .iter()
                            .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
                            .collect(),
                    ),
                );
            }
            JsonValue::Object(map)
        }
        CatalogMcpTransport::Http => {
            let mut map = JsonMap::new();
            map.insert(
                "url".to_string(),
                JsonValue::String(endpoint.url.clone().unwrap_or_default()),
            );
            if !endpoint.headers.is_empty() {
                map.insert(
                    "headers".to_string(),
                    JsonValue::Object(
                        endpoint
                            .headers
                            .iter()
                            .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
                            .collect(),
                    ),
                );
            }
            JsonValue::Object(map)
        }
    }
}

fn upsert_managed_block(
    existing: &str,
    start_marker: &str,
    end_marker: &str,
    block: &str,
) -> String {
    if let Some(start) = existing.find(start_marker)
        && let Some(end_relative) = existing[start..].find(end_marker)
    {
        let end = start + end_relative + end_marker.len();
        let mut updated = String::new();
        updated.push_str(existing[..start].trim_end());
        if !updated.is_empty() {
            updated.push_str("\n\n");
        }
        updated.push_str(block);
        updated.push_str(existing[end..].trim_start_matches(['\r', '\n']));
        return updated.trim_end().to_string() + "\n";
    }

    if existing.trim().is_empty() {
        format!("{block}\n")
    } else {
        format!("{}\n\n{block}\n", existing.trim_end())
    }
}

fn normalize_json(input: &str) -> String {
    serde_json::from_str::<JsonValue>(input)
        .map(|value| serde_json::to_string(&value).unwrap_or_else(|_| input.to_string()))
        .unwrap_or_else(|_| input.trim().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use openagents_core::{
        CatalogInstallRecipe, CatalogItemKind, CatalogItemRecord, CatalogManagedFile,
        CatalogMcpEndpoint, CatalogMcpRecipe, CatalogMcpTransport, CatalogTrustLevel, MemoryConfig,
        ProfileScope, ResolvedProfile, ToolConfig, ToolKind,
    };
    use tempfile::tempdir;

    use super::{ToolSyncContext, reconcile_tool_configs};

    #[test]
    fn reconcile_tool_configs_preserves_user_content_and_installs_skills() {
        let temp = tempdir().expect("temp dir should exist");
        let home = temp.path();
        std::fs::create_dir_all(home.join(".codex")).expect("codex dir should exist");
        std::fs::write(home.join(".codex/config.toml"), "model = \"gpt-5.4\"\n")
            .expect("codex config should write");
        std::fs::write(home.join(".claude.json"), "{ \"theme\": \"dark\" }")
            .expect("claude json should write");
        std::fs::create_dir_all(home.join(".gemini")).expect("gemini dir should exist");
        std::fs::write(
            home.join(".gemini/settings.json"),
            "{ \"theme\": \"dark\" }",
        )
        .expect("gemini settings should write");

        let mut tools = BTreeMap::new();
        for tool in [ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini] {
            tools.insert(
                tool,
                ToolConfig {
                    enabled: true,
                    guidance_packs: vec![],
                },
            );
        }
        let profile = ResolvedProfile {
            name: "personal-client".to_string(),
            description: None,
            memory: MemoryConfig {
                provider: "filesystem".to_string(),
                endpoint: "/memory".to_string(),
                scope: ProfileScope::Client,
            },
            tools,
            skills: vec!["shared-memory".to_string()],
            mcp_servers: vec!["filesystem-memory".to_string()],
        };

        let catalog = BTreeMap::from([
            (
                "shared-memory".to_string(),
                CatalogItemRecord {
                    id: "shared-memory".to_string(),
                    kind: CatalogItemKind::Skill,
                    name: "Shared Memory".to_string(),
                    description: "Skill".to_string(),
                    supported_tools: vec![ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini],
                    trust: CatalogTrustLevel::Vetted,
                    source: "openagents".to_string(),
                    install: CatalogInstallRecipe {
                        managed_files: vec![CatalogManagedFile {
                            relative_path: "skills/shared-memory.md".to_string(),
                            body: "# Shared Memory".to_string(),
                        }],
                        mcp: None,
                    },
                },
            ),
            (
                "filesystem-memory".to_string(),
                CatalogItemRecord {
                    id: "filesystem-memory".to_string(),
                    kind: CatalogItemKind::Mcp,
                    name: "Filesystem Memory".to_string(),
                    description: "MCP".to_string(),
                    supported_tools: vec![ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini],
                    trust: CatalogTrustLevel::Vetted,
                    source: "openagents".to_string(),
                    install: CatalogInstallRecipe {
                        managed_files: vec![],
                        mcp: Some(CatalogMcpRecipe {
                            codex: Some(CatalogMcpEndpoint {
                                transport: CatalogMcpTransport::Stdio,
                                command: Some("npx".to_string()),
                                args: vec!["-y".to_string(), "server".to_string()],
                                url: None,
                                env: BTreeMap::new(),
                                headers: BTreeMap::new(),
                            }),
                            claude: Some(CatalogMcpEndpoint {
                                transport: CatalogMcpTransport::Stdio,
                                command: Some("npx".to_string()),
                                args: vec!["-y".to_string(), "server".to_string()],
                                url: None,
                                env: BTreeMap::new(),
                                headers: BTreeMap::new(),
                            }),
                            gemini: Some(CatalogMcpEndpoint {
                                transport: CatalogMcpTransport::Stdio,
                                command: Some("npx".to_string()),
                                args: vec!["-y".to_string(), "server".to_string()],
                                url: None,
                                env: BTreeMap::new(),
                                headers: BTreeMap::new(),
                            }),
                        }),
                    },
                },
            ),
        ]);

        let summaries = reconcile_tool_configs(
            home,
            &ToolSyncContext {
                workspace_name: "openagents-home".to_string(),
                profile_name: "personal-client".to_string(),
                memory_provider: "filesystem".to_string(),
                memory_endpoint: "/memory".to_string(),
            },
            &profile,
            &catalog,
            false,
        )
        .expect("tool sync should succeed");

        assert_eq!(summaries.len(), 3);
        let codex_contents =
            std::fs::read_to_string(home.join(".codex/config.toml")).expect("codex config");
        assert!(codex_contents.contains("model = \"gpt-5.4\""));
        assert!(codex_contents.contains("# OPENAGENTS BEGIN"));
        assert!(codex_contents.contains("[mcp_servers.\"filesystem-memory\"]"));

        let claude_contents =
            std::fs::read_to_string(home.join(".claude.json")).expect("claude json");
        assert!(claude_contents.contains("\"theme\": \"dark\""));
        assert!(claude_contents.contains("\"openAgents\""));
        assert!(home.join(".claude/commands/shared-memory.md").exists());

        let gemini_contents =
            std::fs::read_to_string(home.join(".gemini/settings.json")).expect("gemini settings");
        assert!(gemini_contents.contains("\"theme\": \"dark\""));
        assert!(gemini_contents.contains("\"mcpServers\""));
        assert!(home.join(".gemini/extensions/shared-memory.md").exists());
    }
}
