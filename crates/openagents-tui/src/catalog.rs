use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use openagents_core::{
    CatalogInstallRecipe, CatalogItemKind, CatalogItemRecord, CatalogManagedFile,
    CatalogMcpEndpoint, CatalogMcpRecipe, CatalogMcpTransport, CatalogTrustLevel,
    CustomCatalogItem, ToolKind,
};
use serde::{Deserialize, Serialize};

const DEFAULT_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/cccuong-jason/openagents-kit/main/catalog/index.json";

#[derive(Debug, Clone, Default)]
pub struct CatalogRegistry {
    pub items: BTreeMap<String, CatalogItemRecord>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogInstallSummary {
    pub skill_paths: Vec<PathBuf>,
    pub mcp_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct CatalogInstallContext {
    pub workspace_name: String,
    pub config_root: PathBuf,
    pub managed_root: PathBuf,
    pub memory_root: PathBuf,
    pub attached_project_root: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CatalogFeed {
    #[serde(default)]
    items: Vec<CatalogItemRecord>,
}

const ALL_TOOLS: &[ToolKind] = &[ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini];

pub fn load_catalog_registry(
    config_root: &Path,
    custom_catalog: &BTreeMap<String, CustomCatalogItem>,
    refresh: bool,
    feed_url_override: Option<&str>,
) -> Result<CatalogRegistry> {
    let cache_path = config_root.join("catalog-cache.json");
    let mut registry = CatalogRegistry {
        items: builtin_items()
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect(),
        warnings: Vec::new(),
    };

    let feed_url = feed_url_override
        .map(str::to_string)
        .or_else(|| std::env::var("OPENAGENTS_CATALOG_URL").ok())
        .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string());

    if refresh {
        match fetch_catalog_feed(&feed_url) {
            Ok(feed) => {
                std::fs::create_dir_all(config_root)?;
                std::fs::write(&cache_path, serde_json::to_vec_pretty(&feed)?)?;
                merge_feed(&mut registry.items, feed.items);
            }
            Err(error) => registry
                .warnings
                .push(format!("catalog refresh failed from {feed_url}: {error}")),
        }
    } else if cache_path.exists() {
        match std::fs::read(&cache_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CatalogFeed>(&bytes).ok())
        {
            Some(feed) => merge_feed(&mut registry.items, feed.items),
            None => registry.warnings.push(format!(
                "catalog cache at {} could not be loaded",
                cache_path.display()
            )),
        }
    }

    for (id, item) in custom_catalog {
        registry.items.insert(
            id.clone(),
            CatalogItemRecord {
                id: id.clone(),
                kind: item.kind,
                name: item.name.clone().unwrap_or_else(|| id.clone()),
                description: item.description.clone(),
                supported_tools: item.supported_tools.clone(),
                trust: CatalogTrustLevel::Custom,
                source: "custom".to_string(),
                install: item
                    .install
                    .clone()
                    .unwrap_or_else(|| CatalogInstallRecipe {
                        managed_files: vec![CatalogManagedFile {
                            relative_path: match item.kind {
                                CatalogItemKind::Skill => format!("skills/{id}.md"),
                                CatalogItemKind::Mcp => format!("mcp/{id}.yaml"),
                            },
                            body: format!(
                                "# Custom {}\n\n{}\n\nInstall summary: {}\n",
                                match item.kind {
                                    CatalogItemKind::Skill => "Skill",
                                    CatalogItemKind::Mcp => "MCP Server",
                                },
                                item.description,
                                item.install_summary
                            ),
                        }],
                        mcp: None,
                    }),
            },
        );
    }

    Ok(registry)
}

pub fn recommended_skill_ids(profile_name: &str) -> Vec<String> {
    match profile_name {
        "team-workspace" => vec!["shared-memory", "team-handoff"],
        "project-sandbox" => vec!["shared-memory", "starter-guidance"],
        _ => vec!["shared-memory", "starter-guidance"],
    }
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn recommended_mcp_ids(profile_name: &str, memory_provider: &str) -> Vec<String> {
    let mut items = match profile_name {
        "team-workspace" => vec!["repo-index", "context7"],
        "project-sandbox" => vec!["repo-index"],
        _ => vec!["repo-index", "context7"],
    }
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    if memory_provider == "filesystem" {
        items.insert(0, "filesystem-memory".to_string());
    }

    items.sort();
    items.dedup();
    items
}

pub fn install_catalog_assets(
    registry: &CatalogRegistry,
    install_context: &CatalogInstallContext,
    skills: &[String],
    mcp_servers: &[String],
    dry_run: bool,
) -> Result<CatalogInstallSummary> {
    let mut summary = CatalogInstallSummary::default();

    for id in skills.iter().chain(mcp_servers.iter()) {
        let Some(item) = registry.items.get(id) else {
            continue;
        };

        for managed_file in &item.install.managed_files {
            let target_path = install_context.managed_root.join(interpolate_placeholders(
                &managed_file.relative_path,
                install_context,
            ));
            match item.kind {
                CatalogItemKind::Skill => summary.skill_paths.push(target_path.clone()),
                CatalogItemKind::Mcp => summary.mcp_paths.push(target_path.clone()),
            }

            if dry_run {
                continue;
            }

            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &target_path,
                interpolate_placeholders(&managed_file.body, install_context),
            )?;
        }
    }

    Ok(summary)
}

pub fn interpolate_placeholders(template: &str, context: &CatalogInstallContext) -> String {
    template
        .replace("${workspace_name}", &context.workspace_name)
        .replace("${config_root}", &context.config_root.display().to_string())
        .replace(
            "${managed_root}",
            &context.managed_root.display().to_string(),
        )
        .replace("${memory_root}", &context.memory_root.display().to_string())
        .replace(
            "${attached_project_root}",
            &context.attached_project_root.display().to_string(),
        )
}

fn merge_feed(target: &mut BTreeMap<String, CatalogItemRecord>, items: Vec<CatalogItemRecord>) {
    for item in items {
        target.insert(item.id.clone(), item);
    }
}

fn fetch_catalog_feed(url: &str) -> Result<CatalogFeed> {
    if let Some(path) = url.strip_prefix("file://") {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read catalog feed from {path}"))?;
        return Ok(serde_json::from_slice(&bytes)?);
    }

    let response = reqwest::blocking::get(url)
        .with_context(|| format!("failed to download catalog feed from {url}"))?
        .error_for_status()
        .with_context(|| format!("catalog feed request failed for {url}"))?;
    Ok(response.json()?)
}

fn builtin_items() -> Vec<CatalogItemRecord> {
    vec![
        skill_item(
            "shared-memory",
            "Shared Memory",
            "Starter guidance for keeping cross-tool context consistent.",
            CatalogTrustLevel::Vetted,
            "openagents",
            "# Shared Memory\n\nKeep working notes, decisions, and handoffs in one place so Codex, Claude, and Gemini stay aligned.\n",
        ),
        skill_item(
            "starter-guidance",
            "Starter Guidance",
            "Plain-language prompts for onboarding non-technical clients.",
            CatalogTrustLevel::Vetted,
            "openagents",
            "# Starter Guidance\n\nExplain what OpenAgents is doing, why it matters, and what the user should do next.\n",
        ),
        skill_item(
            "team-handoff",
            "Team Handoff",
            "Checklist for longer-lived workspaces shared across people and devices.",
            CatalogTrustLevel::Vetted,
            "openagents",
            "# Team Handoff\n\nCapture owner, next actions, and sync status before switching tools or devices.\n",
        ),
        CatalogItemRecord {
            id: "filesystem-memory".to_string(),
            kind: CatalogItemKind::Mcp,
            name: "Filesystem Memory".to_string(),
            description: "Expose the OpenAgents memory directory through a filesystem MCP server."
                .to_string(),
            supported_tools: ALL_TOOLS.to_vec(),
            trust: CatalogTrustLevel::Vetted,
            source: "openagents".to_string(),
            install: CatalogInstallRecipe {
                managed_files: vec![CatalogManagedFile {
                    relative_path: "mcp/filesystem-memory.yaml".to_string(),
                    body: "name: filesystem-memory\ntrust: vetted\nsource: openagents\nmemory_root: ${memory_root}\n".to_string(),
                }],
                mcp: Some(unified_stdio_mcp(vec![
                    "npx",
                    "-y",
                    "@modelcontextprotocol/server-filesystem",
                    "${memory_root}",
                ])),
            },
        },
        CatalogItemRecord {
            id: "repo-index".to_string(),
            kind: CatalogItemKind::Mcp,
            name: "Repo Index".to_string(),
            description: "Expose the attached project through an MCP filesystem server for repo-aware navigation.".to_string(),
            supported_tools: ALL_TOOLS.to_vec(),
            trust: CatalogTrustLevel::Community,
            source: "openagents-community".to_string(),
            install: CatalogInstallRecipe {
                managed_files: vec![CatalogManagedFile {
                    relative_path: "mcp/repo-index.yaml".to_string(),
                    body: "name: repo-index\ntrust: community\nsource: openagents-community\nattached_project_root: ${attached_project_root}\n".to_string(),
                }],
                mcp: Some(unified_stdio_mcp(vec![
                    "npx",
                    "-y",
                    "@modelcontextprotocol/server-filesystem",
                    "${attached_project_root}",
                ])),
            },
        },
        CatalogItemRecord {
            id: "context7".to_string(),
            kind: CatalogItemKind::Mcp,
            name: "Context7".to_string(),
            description: "Community docs retriever for coding assistants.".to_string(),
            supported_tools: ALL_TOOLS.to_vec(),
            trust: CatalogTrustLevel::Community,
            source: "upstash/context7".to_string(),
            install: CatalogInstallRecipe {
                managed_files: vec![CatalogManagedFile {
                    relative_path: "mcp/context7.yaml".to_string(),
                    body: "name: context7\ntrust: community\nsource: upstash/context7\n".to_string(),
                }],
                mcp: Some(unified_stdio_mcp(vec![
                    "npx",
                    "-y",
                    "@buggyhunter/context7-mcp",
                ])),
            },
        },
    ]
}

pub fn curated_items() -> Vec<CatalogItemRecord> {
    builtin_items()
}

fn skill_item(
    id: &str,
    name: &str,
    description: &str,
    trust: CatalogTrustLevel,
    source: &str,
    body: &str,
) -> CatalogItemRecord {
    CatalogItemRecord {
        id: id.to_string(),
        kind: CatalogItemKind::Skill,
        name: name.to_string(),
        description: description.to_string(),
        supported_tools: ALL_TOOLS.to_vec(),
        trust,
        source: source.to_string(),
        install: CatalogInstallRecipe {
            managed_files: vec![CatalogManagedFile {
                relative_path: format!("skills/{id}.md"),
                body: body.to_string(),
            }],
            mcp: None,
        },
    }
}

fn unified_stdio_mcp(args: Vec<&str>) -> CatalogMcpRecipe {
    let command = args
        .first()
        .expect("stdio recipe requires command")
        .to_string();
    let args = args
        .into_iter()
        .skip(1)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let endpoint = CatalogMcpEndpoint {
        transport: CatalogMcpTransport::Stdio,
        command: Some(command),
        args,
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
    };
    CatalogMcpRecipe {
        codex: Some(endpoint.clone()),
        claude: Some(endpoint.clone()),
        gemini: Some(endpoint),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use openagents_core::{CatalogItemKind, CatalogTrustLevel};
    use tempfile::tempdir;

    use super::{
        CatalogInstallContext, CatalogInstallSummary, install_catalog_assets,
        interpolate_placeholders, load_catalog_registry, recommended_mcp_ids,
    };

    #[test]
    fn recommends_filesystem_memory_mcp_when_local_memory_is_selected() {
        let mcps = recommended_mcp_ids("personal-client", "filesystem");

        assert!(mcps.contains(&"filesystem-memory".to_string()));
        assert!(mcps.contains(&"repo-index".to_string()));
    }

    #[test]
    fn installs_curated_assets_into_managed_root() {
        let temp = tempdir().expect("temp dir should exist");
        let registry = load_catalog_registry(temp.path(), &BTreeMap::new(), false, None)
            .expect("catalog should load");
        let summary = install_catalog_assets(
            &registry,
            &CatalogInstallContext {
                workspace_name: "openagents-home".to_string(),
                config_root: temp.path().to_path_buf(),
                managed_root: temp.path().join("managed"),
                memory_root: temp.path().join("memory"),
                attached_project_root: temp.path().join("repo"),
            },
            &["shared-memory".to_string()],
            &["filesystem-memory".to_string()],
            false,
        )
        .expect("catalog assets should install");

        assert_eq!(summary.skill_paths.len(), 1);
        assert_eq!(summary.mcp_paths.len(), 1);
        assert!(summary.skill_paths[0].exists());
        assert!(summary.mcp_paths[0].exists());
    }

    #[test]
    fn dry_run_catalog_install_reports_paths_without_writing() {
        let temp = tempdir().expect("temp dir should exist");
        let registry = load_catalog_registry(temp.path(), &BTreeMap::new(), false, None)
            .expect("catalog should load");
        let summary = install_catalog_assets(
            &registry,
            &CatalogInstallContext {
                workspace_name: "openagents-home".to_string(),
                config_root: temp.path().to_path_buf(),
                managed_root: temp.path().join("managed"),
                memory_root: temp.path().join("memory"),
                attached_project_root: temp.path().join("repo"),
            },
            &["starter-guidance".to_string()],
            &[],
            true,
        )
        .expect("catalog assets should dry-run");

        assert_eq!(
            summary,
            CatalogInstallSummary {
                skill_paths: vec![
                    temp.path()
                        .join("managed")
                        .join("skills")
                        .join("starter-guidance.md")
                ],
                mcp_paths: vec![],
            }
        );
        assert!(!temp.path().join("managed").join("skills").exists());
    }

    #[test]
    fn loads_remote_catalog_feed_and_caches_it() {
        let temp = tempdir().expect("temp dir should exist");
        let feed_path = temp.path().join("feed.json");
        fs::write(
            &feed_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "items": [{
                    "id": "playwright-mcp",
                    "kind": "mcp",
                    "name": "Playwright MCP",
                    "description": "Browser automation MCP.",
                    "supported_tools": ["codex", "claude", "gemini"],
                    "trust": "community",
                    "source": "remote-community",
                    "install": {
                        "managed_files": [{
                            "relative_path": "mcp/playwright-mcp.yaml",
                            "body": "name: playwright-mcp\n"
                        }]
                    }
                }]
            }))
            .expect("feed should serialize"),
        )
        .expect("feed should write");

        let registry = load_catalog_registry(
            temp.path(),
            &BTreeMap::new(),
            true,
            Some(&format!("file://{}", feed_path.display())),
        )
        .expect("catalog should refresh");

        let item = registry
            .items
            .get("playwright-mcp")
            .expect("remote item should be present");
        assert_eq!(item.kind, CatalogItemKind::Mcp);
        assert_eq!(item.trust, CatalogTrustLevel::Community);
        assert!(temp.path().join("catalog-cache.json").exists());
    }

    #[test]
    fn interpolates_install_placeholders() {
        let rendered = interpolate_placeholders(
            "${workspace_name} ${memory_root} ${attached_project_root}",
            &CatalogInstallContext {
                workspace_name: "openagents-home".to_string(),
                config_root: PathBuf::from("/config"),
                managed_root: PathBuf::from("/managed"),
                memory_root: PathBuf::from("/memory"),
                attached_project_root: PathBuf::from("/repo"),
            },
        );

        assert_eq!(rendered, "openagents-home /memory /repo");
    }

    #[test]
    fn custom_catalog_items_are_normalized_into_the_registry() {
        let temp = tempdir().expect("temp dir should exist");
        let mut custom = BTreeMap::new();
        custom.insert(
            "my-skill".to_string(),
            openagents_core::CustomCatalogItem {
                kind: CatalogItemKind::Skill,
                name: Some("My Skill".to_string()),
                description: "Custom skill".to_string(),
                supported_tools: vec![],
                install_summary: "Copy into tool prompts".to_string(),
                install: None,
            },
        );

        let registry =
            load_catalog_registry(temp.path(), &custom, false, None).expect("catalog should load");

        let item = registry
            .items
            .get("my-skill")
            .expect("custom item should exist");
        assert_eq!(item.trust, CatalogTrustLevel::Custom);
        assert_eq!(item.name, "My Skill");
    }
}
