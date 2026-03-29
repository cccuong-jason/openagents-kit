use std::path::{Path, PathBuf};

use openagents_core::ToolKind;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDetection {
    pub tool: ToolKind,
    pub evidence_path: PathBuf,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectionReport {
    pub detections: Vec<ToolDetection>,
    pub warnings: Vec<String>,
    pub installed_skills: Vec<String>,
    pub installed_mcp_servers: Vec<String>,
    pub has_memory_layer: bool,
}

pub fn detect_tools_in_home(home: &Path) -> DetectionReport {
    let mut report = DetectionReport::default();

    detect_codex(home, &mut report);
    detect_claude(home, &mut report);
    detect_gemini(home, &mut report);
    detect_openagents_memory(home, &mut report);

    report.detections.sort_by_key(|item| item.tool.to_string());
    report.installed_skills.sort();
    report.installed_skills.dedup();
    report.installed_mcp_servers.sort();
    report.installed_mcp_servers.dedup();

    report
}

fn detect_codex(home: &Path, report: &mut DetectionReport) {
    let config_path = home.join(".codex/config.toml");
    if !config_path.exists() {
        return;
    }

    let mut summary = "Codex config found".to_string();
    match std::fs::read_to_string(&config_path) {
        Ok(contents) => match contents.parse::<TomlValue>() {
            Ok(parsed) => {
                if let Some(model) = parsed.get("model").and_then(TomlValue::as_str) {
                    summary = format!("Codex config found ({model})");
                }

                if let Some(mcp_servers) = parsed.get("mcp_servers").and_then(TomlValue::as_table) {
                    for key in mcp_servers.keys() {
                        push_unique(&mut report.installed_mcp_servers, key);
                    }
                }
            }
            Err(error) => report.warnings.push(format!(
                "Codex config was detected at {} but could not be parsed: {error}",
                config_path.display()
            )),
        },
        Err(error) => report.warnings.push(format!(
            "Codex config was detected at {} but could not be read: {error}",
            config_path.display()
        )),
    }

    report.detections.push(ToolDetection {
        tool: ToolKind::Codex,
        evidence_path: config_path,
        summary,
    });
}

fn detect_claude(home: &Path, report: &mut DetectionReport) {
    let state_path = home.join(".claude.json");
    if !state_path.exists() {
        return;
    }

    let mut summary = "Claude state found".to_string();
    match std::fs::read_to_string(&state_path) {
        Ok(contents) => match serde_json::from_str::<JsonValue>(&contents) {
            Ok(parsed) => {
                if let Some(first_start) = parsed.get("firstStartTime").and_then(JsonValue::as_str)
                {
                    summary = format!("Claude state found (started {first_start})");
                }

                detect_json_mcp_keys(&parsed, report);
            }
            Err(error) => report.warnings.push(format!(
                "Claude state was detected at {} but could not be parsed: {error}",
                state_path.display()
            )),
        },
        Err(error) => report.warnings.push(format!(
            "Claude state was detected at {} but could not be read: {error}",
            state_path.display()
        )),
    }

    let commands_dir = home.join(".claude/commands");
    if commands_dir.exists() {
        detect_skill_files(&commands_dir, &mut report.installed_skills);
    }

    report.detections.push(ToolDetection {
        tool: ToolKind::Claude,
        evidence_path: state_path,
        summary,
    });
}

fn detect_gemini(home: &Path, report: &mut DetectionReport) {
    let settings_path = home.join(".gemini/settings.json");
    let state_path = home.join(".gemini/state.json");
    let evidence_path = if settings_path.exists() {
        settings_path
    } else if state_path.exists() {
        state_path
    } else {
        return;
    };

    let mut summary = "Gemini settings found".to_string();
    match std::fs::read_to_string(&evidence_path) {
        Ok(contents) => match serde_json::from_str::<JsonValue>(&contents) {
            Ok(parsed) => {
                if let Some(selected_type) = parsed
                    .get("security")
                    .and_then(|value| value.get("auth"))
                    .and_then(|value| value.get("selectedType"))
                    .and_then(JsonValue::as_str)
                {
                    summary = format!("Gemini settings found ({selected_type})");
                }
                detect_json_mcp_keys(&parsed, report);
            }
            Err(error) => report.warnings.push(format!(
                "Gemini settings were detected at {} but could not be parsed: {error}",
                evidence_path.display()
            )),
        },
        Err(error) => report.warnings.push(format!(
            "Gemini settings were detected at {} but could not be read: {error}",
            evidence_path.display()
        )),
    }

    let extensions_dir = home.join(".gemini/extensions");
    if extensions_dir.exists() {
        detect_skill_files(&extensions_dir, &mut report.installed_skills);
    }

    report.detections.push(ToolDetection {
        tool: ToolKind::Gemini,
        evidence_path,
        summary,
    });
}

fn detect_openagents_memory(home: &Path, report: &mut DetectionReport) {
    if home.join(".openagents/memory").exists() {
        report.has_memory_layer = true;
    }
}

fn detect_skill_files(root: &Path, target: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        push_unique(target, &sanitize_id(stem));
    }
}

fn detect_json_mcp_keys(value: &JsonValue, report: &mut DetectionReport) {
    let candidates = [
        value.get("mcpServers"),
        value.get("mcp_servers"),
        value.get("mcp"),
    ];

    for candidate in candidates.into_iter().flatten() {
        if let Some(map) = candidate.as_object() {
            for key in map.keys() {
                push_unique(&mut report.installed_mcp_servers, key);
            }
        }
    }
}

fn sanitize_id(value: &str) -> String {
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

fn push_unique(target: &mut Vec<String>, value: &str) {
    if !target.iter().any(|item| item == value) {
        target.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::detect_tools_in_home;
    use openagents_core::ToolKind;

    #[test]
    fn detects_tools_from_known_config_paths() {
        let temp = tempdir().expect("temp dir should exist");
        let home = temp.path();

        fs::create_dir_all(home.join(".codex")).expect("codex dir should exist");
        fs::write(
            home.join(".codex/config.toml"),
            "model = \"gpt-5.4\"\n[mcp_servers]\nfilesystem-memory = { command = \"openagents-kit\" }\n",
        )
        .expect("codex config should exist");
        fs::write(
            home.join(".claude.json"),
            "{ \"firstStartTime\": \"2026-03-26\", \"mcpServers\": { \"context7\": {} } }",
        )
        .expect("claude state should exist");
        fs::create_dir_all(home.join(".claude/commands"))
            .expect("claude commands dir should exist");
        fs::write(home.join(".claude/commands/shared-memory.md"), "# skill")
            .expect("claude command should exist");
        fs::create_dir_all(home.join(".gemini")).expect("gemini dir should exist");
        fs::write(
            home.join(".gemini/settings.json"),
            "{ \"security\": { \"auth\": { \"selectedType\": \"oauth-personal\" } }, \"mcpServers\": { \"repo-index\": {} } }",
        )
        .expect("gemini settings should exist");
        fs::create_dir_all(home.join(".gemini/extensions"))
            .expect("gemini extensions dir should exist");
        fs::write(home.join(".gemini/extensions/team-handoff.md"), "# skill")
            .expect("gemini extension should exist");

        let report = detect_tools_in_home(home);

        assert_eq!(report.detections.len(), 3);
        assert!(
            report
                .detections
                .iter()
                .any(|item| item.tool == ToolKind::Codex)
        );
        assert!(
            report
                .installed_skills
                .contains(&"shared-memory".to_string())
        );
        assert!(
            report
                .installed_skills
                .contains(&"team-handoff".to_string())
        );
        assert!(
            report
                .installed_mcp_servers
                .contains(&"filesystem-memory".to_string())
        );
        assert!(
            report
                .installed_mcp_servers
                .contains(&"context7".to_string())
        );
        assert!(
            report
                .installed_mcp_servers
                .contains(&"repo-index".to_string())
        );
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn reports_parse_warning_but_keeps_detection() {
        let temp = tempdir().expect("temp dir should exist");
        let home = temp.path();

        fs::create_dir_all(home.join(".codex")).expect("codex dir should exist");
        fs::write(home.join(".codex/config.toml"), "not valid toml =")
            .expect("codex config should exist");

        let report = detect_tools_in_home(home);

        assert_eq!(report.detections.len(), 1);
        assert_eq!(report.detections[0].tool, ToolKind::Codex);
        assert_eq!(report.warnings.len(), 1);
    }
}
