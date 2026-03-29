use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use openagents_core::{CatalogItemKind, CustomCatalogItem, ToolKind};

#[derive(Debug, Clone)]
pub struct CatalogItem {
    pub id: &'static str,
    pub kind: CatalogItemKind,
    pub name: &'static str,
    pub description: &'static str,
    pub supported_tools: &'static [ToolKind],
    pub install_filename: &'static str,
    pub install_body: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogInstallSummary {
    pub skill_paths: Vec<PathBuf>,
    pub mcp_paths: Vec<PathBuf>,
}

const ALL_TOOLS: &[ToolKind] = &[ToolKind::Codex, ToolKind::Claude, ToolKind::Gemini];

const CATALOG_ITEMS: &[CatalogItem] = &[
    CatalogItem {
        id: "shared-memory",
        kind: CatalogItemKind::Skill,
        name: "Shared Memory",
        description: "Starter guidance for keeping cross-tool context consistent.",
        supported_tools: ALL_TOOLS,
        install_filename: "shared-memory.md",
        install_body: "# Shared Memory\n\nKeep working notes, decisions, and handoffs in one place so Codex, Claude, and Gemini stay aligned.\n",
    },
    CatalogItem {
        id: "starter-guidance",
        kind: CatalogItemKind::Skill,
        name: "Starter Guidance",
        description: "Plain-language prompts for onboarding non-technical clients.",
        supported_tools: ALL_TOOLS,
        install_filename: "starter-guidance.md",
        install_body: "# Starter Guidance\n\nExplain what OpenAgents is doing, why it matters, and what the user should do next.\n",
    },
    CatalogItem {
        id: "team-handoff",
        kind: CatalogItemKind::Skill,
        name: "Team Handoff",
        description: "Checklist for longer-lived workspaces shared across people and devices.",
        supported_tools: ALL_TOOLS,
        install_filename: "team-handoff.md",
        install_body: "# Team Handoff\n\nCapture owner, next actions, and sync status before switching tools or devices.\n",
    },
    CatalogItem {
        id: "filesystem-memory",
        kind: CatalogItemKind::Mcp,
        name: "Filesystem Memory",
        description: "Local memory layer stored inside the OpenAgents config root.",
        supported_tools: ALL_TOOLS,
        install_filename: "filesystem-memory.yaml",
        install_body: "name: filesystem-memory\nkind: managed\npurpose: Local memory store rooted in the OpenAgents config directory.\n",
    },
    CatalogItem {
        id: "context7",
        kind: CatalogItemKind::Mcp,
        name: "Context7",
        description: "Reference retriever for docs and technical context.",
        supported_tools: ALL_TOOLS,
        install_filename: "context7.yaml",
        install_body: "name: context7\nkind: companion\npurpose: Fetches external documentation and reference context when available.\n",
    },
    CatalogItem {
        id: "repo-index",
        kind: CatalogItemKind::Mcp,
        name: "Repo Index",
        description: "Repo-aware search and indexing helper for larger projects.",
        supported_tools: ALL_TOOLS,
        install_filename: "repo-index.yaml",
        install_body: "name: repo-index\nkind: companion\npurpose: Provides repo indexing and semantic search hints for managed tools.\n",
    },
];

pub fn curated_items() -> &'static [CatalogItem] {
    CATALOG_ITEMS
}

pub fn find_curated_item(id: &str) -> Option<&'static CatalogItem> {
    curated_items().iter().find(|item| item.id == id)
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
        "team-workspace" => vec!["context7", "repo-index"],
        "project-sandbox" => vec!["repo-index"],
        _ => vec!["context7"],
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
    managed_root: &Path,
    skills: &[String],
    mcp_servers: &[String],
    custom_catalog: &BTreeMap<String, CustomCatalogItem>,
    dry_run: bool,
) -> Result<CatalogInstallSummary> {
    let skills_root = managed_root.join("skills");
    let mcp_root = managed_root.join("mcp");
    let mut summary = CatalogInstallSummary::default();

    for skill in skills {
        let path = skills_root.join(asset_file_name(
            skill,
            CatalogItemKind::Skill,
            custom_catalog,
        ));
        summary.skill_paths.push(path.clone());
        if dry_run {
            continue;
        }
        std::fs::create_dir_all(&skills_root)?;
        std::fs::write(
            &path,
            asset_body(skill, CatalogItemKind::Skill, custom_catalog),
        )?;
    }

    for mcp in mcp_servers {
        let path = mcp_root.join(asset_file_name(mcp, CatalogItemKind::Mcp, custom_catalog));
        summary.mcp_paths.push(path.clone());
        if dry_run {
            continue;
        }
        std::fs::create_dir_all(&mcp_root)?;
        std::fs::write(&path, asset_body(mcp, CatalogItemKind::Mcp, custom_catalog))?;
    }

    Ok(summary)
}

fn asset_file_name(
    id: &str,
    kind: CatalogItemKind,
    custom_catalog: &BTreeMap<String, CustomCatalogItem>,
) -> String {
    if let Some(item) = find_curated_item(id) {
        return item.install_filename.to_string();
    }

    let extension = match kind {
        CatalogItemKind::Skill => "md",
        CatalogItemKind::Mcp => "yaml",
    };

    if custom_catalog.contains_key(id) {
        return format!("{id}.{extension}");
    }

    format!("{id}.{extension}")
}

fn asset_body(
    id: &str,
    kind: CatalogItemKind,
    custom_catalog: &BTreeMap<String, CustomCatalogItem>,
) -> String {
    if let Some(item) = find_curated_item(id) {
        return item.install_body.to_string();
    }

    if let Some(item) = custom_catalog.get(id) {
        return format!(
            "# Custom {}\n\n{}\n\nInstall summary: {}\n",
            match kind {
                CatalogItemKind::Skill => "Skill",
                CatalogItemKind::Mcp => "MCP Server",
            },
            item.description,
            item.install_summary
        );
    }

    format!(
        "# Unknown {}\n\nOpenAgents expected `{id}` to exist in the catalog but it was missing.\n",
        match kind {
            CatalogItemKind::Skill => "Skill",
            CatalogItemKind::Mcp => "MCP Server",
        }
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::{CatalogInstallSummary, install_catalog_assets, recommended_mcp_ids};

    #[test]
    fn recommends_filesystem_memory_mcp_when_local_memory_is_selected() {
        let mcps = recommended_mcp_ids("personal-client", "filesystem");

        assert!(mcps.contains(&"filesystem-memory".to_string()));
        assert!(mcps.contains(&"context7".to_string()));
    }

    #[test]
    fn installs_curated_assets_into_managed_root() {
        let temp = tempdir().expect("temp dir should exist");
        let summary = install_catalog_assets(
            temp.path(),
            &["shared-memory".to_string()],
            &["filesystem-memory".to_string()],
            &BTreeMap::new(),
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
        let summary = install_catalog_assets(
            temp.path(),
            &["starter-guidance".to_string()],
            &[],
            &BTreeMap::new(),
            true,
        )
        .expect("catalog assets should dry-run");

        assert_eq!(
            summary,
            CatalogInstallSummary {
                skill_paths: vec![temp.path().join("skills").join("starter-guidance.md")],
                mcp_paths: vec![],
            }
        );
        assert!(!temp.path().join("skills").exists());
    }
}
