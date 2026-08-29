use crate::models::{AgentIntegrationInfo, AgentIntegrationResult};
use std::path::{Path, PathBuf};

const ZENITH_HOOK_ID: &str = "zenith-agent-activity";

pub fn get_integration_info(tool_id: &str, home_dir: &Path) -> AgentIntegrationInfo {
    let (display_name, supported, config_rel_path, description) = match tool_id {
        "antigravity" => (
            "Antigravity",
            true,
            ".gemini/antigravity/hooks.json",
            "Process-only observation. A verified Zenith event bridge is not available yet.",
        ),
        "claude" => (
            "Claude Code",
            true,
            ".claude/settings.json",
            "Process-only observation. A verified Zenith event bridge is not available yet.",
        ),
        "cursor" => (
            "Cursor Agent CLI",
            true,
            ".cursor/hooks.json",
            "Process-only observation. A verified Zenith event bridge is not available yet.",
        ),
        "grok" => (
            "Grok Build",
            true,
            ".grok/hooks.json",
            "Process-only observation. A verified Zenith event bridge is not available yet.",
        ),
        "copilot" => (
            "GitHub Copilot CLI",
            true,
            ".copilot/hooks.json",
            "Process-only observation. A verified Zenith event bridge is not available yet.",
        ),
        "gemini" => (
            "Gemini CLI (legacy / enterprise)",
            false,
            "",
            "Process-only observation. Individual accounts transitioned to Antigravity CLI.",
        ),
        "codex" => (
            "Codex CLI",
            false,
            "",
            "Process-only observation for unmanaged TUI.",
        ),
        "opencode" => ("OpenCode", false, "", "Process-only observation."),
        _ => ("Unknown tool", false, "", "Unsupported tool."),
    };

    if !supported {
        return AgentIntegrationInfo {
            tool_id: tool_id.to_string(),
            display_name: display_name.to_string(),
            supported: false,
            installed: false,
            integration_active: false,
            config_path: None,
            description: description.to_string(),
        };
    }

    let config_path = home_dir.join(config_rel_path);
    let installed = config_path.exists();
    let integration_active = if installed {
        is_hook_present(&config_path)
    } else {
        false
    };

    AgentIntegrationInfo {
        tool_id: tool_id.to_string(),
        display_name: display_name.to_string(),
        supported: true,
        installed,
        integration_active,
        config_path: Some(config_path.display().to_string()),
        description: description.to_string(),
    }
}

pub fn install_integration(
    tool_id: &str,
    home_dir: &Path,
) -> Result<AgentIntegrationResult, String> {
    let info = get_integration_info(tool_id, home_dir);
    if !info.supported {
        return Err(format!(
            "Tool '{tool_id}' does not support local hook integration."
        ));
    }
    Err(format!(
        "Local integration for {} is unavailable until Zenith ships a verified protocol-specific event bridge.",
        info.display_name
    ))
}

pub fn uninstall_integration(
    tool_id: &str,
    home_dir: &Path,
) -> Result<AgentIntegrationResult, String> {
    let info = get_integration_info(tool_id, home_dir);
    let Some(config_str) = info.config_path else {
        return Err("Configuration path not determined.".to_string());
    };
    let config_path = PathBuf::from(config_str);

    if !config_path.exists() {
        return Ok(AgentIntegrationResult {
            tool_id: tool_id.to_string(),
            success: true,
            message: "Integration was not installed.".to_string(),
        });
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read existing config: {e}"))?;
    let mut json_value: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse config JSON: {e}"))?;

    match tool_id {
        "claude" => {
            if let Some(obj) = json_value.as_object_mut() {
                if let Some(hooks) = obj.get_mut("hooks").and_then(|v| v.as_object_mut()) {
                    hooks.remove(ZENITH_HOOK_ID);
                }
            }
        }
        _ => {
            if let Some(obj) = json_value.as_object_mut() {
                if let Some(hooks) = obj.get_mut("hooks").and_then(|v| v.as_array_mut()) {
                    hooks.retain(|item| {
                        item.get("id").and_then(|v| v.as_str()) != Some(ZENITH_HOOK_ID)
                    });
                }
            }
        }
    }

    atomic_write_json(&config_path, &json_value)?;

    Ok(AgentIntegrationResult {
        tool_id: tool_id.to_string(),
        success: true,
        message: format!("Local integration for {} removed.", info.display_name),
    })
}

fn is_hook_present(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    let Some(hooks) = value.get("hooks") else {
        return false;
    };
    hooks
        .as_object()
        .is_some_and(|object| object.contains_key(ZENITH_HOOK_ID))
        || hooks.as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("id").and_then(|id| id.as_str()) == Some(ZENITH_HOOK_ID))
        })
}

fn atomic_write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|e| format!("Failed to serialize JSON: {e}"))?;
    let temp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&temp_path, serialized)
        .map_err(|e| format!("Failed to write temporary config: {e}"))?;
    std::fs::rename(&temp_path, path)
        .map_err(|e| format!("Failed to atomically replace config file: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_install_an_unverified_vendor_hook() {
        let temp = tempfile::tempdir().unwrap();
        let result = install_integration("antigravity", temp.path());
        assert!(result.unwrap_err().contains("verified protocol-specific"));
        assert!(!temp.path().join(".gemini/antigravity/hooks.json").exists());
    }

    #[test]
    fn removes_a_legacy_zenith_marker_without_touching_user_hooks() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let path = home.join(".claude/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "custom_setting": "preserve_me",
                "hooks": {
                    "user-hook": { "type": "command" },
                    ZENITH_HOOK_ID: { "type": "zenith_local" }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(get_integration_info("claude", home).integration_active);

        let uninstall_res = uninstall_integration("claude", home);
        assert!(uninstall_res.is_ok());
        assert!(!get_integration_info("claude", home).integration_active);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["custom_setting"], "preserve_me");
        assert!(value["hooks"].get("user-hook").is_some());
    }
}
