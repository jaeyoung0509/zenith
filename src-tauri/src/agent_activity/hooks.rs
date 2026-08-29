use crate::models::{AgentIntegrationInfo, AgentIntegrationResult};
use std::path::{Path, PathBuf};

const ZENITH_HOOK_ID: &str = "zenith-agent-activity";

pub fn get_integration_info(tool_id: &str, home_dir: &Path) -> AgentIntegrationInfo {
    let (display_name, supported, config_rel_path, description) = match tool_id {
        "antigravity" => (
            "Antigravity",
            true,
            ".gemini/antigravity/hooks.json",
            "Google's primary agent CLI. Supports lifecycle and status-line integration.",
        ),
        "claude" => (
            "Claude Code",
            true,
            ".claude/settings.json",
            "Claude Code official lifecycle and notification hooks.",
        ),
        "cursor" => (
            "Cursor Agent CLI",
            true,
            ".cursor/hooks.json",
            "Cursor local lifecycle hooks for Agent CLI.",
        ),
        "grok" => (
            "Grok Build",
            true,
            ".grok/hooks.json",
            "xAI Grok Build lifecycle hooks.",
        ),
        "copilot" => (
            "GitHub Copilot CLI",
            true,
            ".copilot/hooks.json",
            "GitHub Copilot CLI lifecycle and notification hooks.",
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
    let Some(config_str) = info.config_path else {
        return Err("Configuration path not determined.".to_string());
    };
    let config_path = PathBuf::from(config_str);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    let mut json_value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read existing config: {e}"))?;
        serde_json::from_str::<serde_json::Value>(&content)
            .unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Inject Zenith hook without overwriting other hooks
    match tool_id {
        "claude" => {
            let hooks_obj = json_value
                .as_object_mut()
                .ok_or_else(|| "Invalid JSON format in Claude config.".to_string())?
                .entry("hooks")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(obj) = hooks_obj.as_object_mut() {
                obj.insert(
                    ZENITH_HOOK_ID.to_string(),
                    serde_json::json!({
                        "events": ["SessionStart", "Stop", "SessionEnd", "Notification"],
                        "type": "zenith_local"
                    }),
                );
            }
        }
        _ => {
            // Array-based hooks (antigravity, cursor, grok, copilot)
            let hooks_arr = json_value
                .as_object_mut()
                .ok_or_else(|| "Invalid JSON format in tool config.".to_string())?
                .entry("hooks")
                .or_insert_with(|| serde_json::json!([]));
            if let Some(arr) = hooks_arr.as_array_mut() {
                arr.retain(|item| item.get("id").and_then(|v| v.as_str()) != Some(ZENITH_HOOK_ID));
                arr.push(serde_json::json!({
                    "id": ZENITH_HOOK_ID,
                    "events": ["SessionStart", "Stop", "SessionEnd"],
                    "type": "zenith_local"
                }));
            }
        }
    }

    atomic_write_json(&config_path, &json_value)?;

    Ok(AgentIntegrationResult {
        tool_id: tool_id.to_string(),
        success: true,
        message: format!(
            "Local integration for {} installed successfully.",
            info.display_name
        ),
    })
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
    content.contains(ZENITH_HOOK_ID)
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
    fn installs_and_uninstalls_preserving_other_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();

        // 1. Initial state: not installed
        let info = get_integration_info("antigravity", home);
        assert!(!info.integration_active);

        // 2. Pre-create config with existing custom user hook
        let config_path = home.join(".gemini/antigravity/hooks.json");
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        let initial_json = serde_json::json!({
            "custom_setting": "preserve_me",
            "hooks": [
                {
                    "id": "user-custom-hook",
                    "events": ["SessionStart"]
                }
            ]
        });
        std::fs::write(
            &config_path,
            serde_json::to_string_pretty(&initial_json).unwrap(),
        )
        .unwrap();

        // 3. Install integration
        let install_res = install_integration("antigravity", home);
        assert!(install_res.is_ok());

        let after_install = get_integration_info("antigravity", home);
        assert!(after_install.integration_active);

        // Verify existing custom hook was preserved!
        let installed_content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(installed_content["custom_setting"], "preserve_me");
        let hooks = installed_content["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), 2);
        assert!(hooks.iter().any(|h| h["id"] == "user-custom-hook"));
        assert!(hooks.iter().any(|h| h["id"] == ZENITH_HOOK_ID));

        // 4. Uninstall integration
        let uninstall_res = uninstall_integration("antigravity", home);
        assert!(uninstall_res.is_ok());

        let after_uninstall = get_integration_info("antigravity", home);
        assert!(!after_uninstall.integration_active);

        // Verify custom hook is still preserved, only Zenith hook removed!
        let final_content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(final_content["custom_setting"], "preserve_me");
        let final_hooks = final_content["hooks"].as_array().unwrap();
        assert_eq!(final_hooks.len(), 1);
        assert_eq!(final_hooks[0]["id"], "user-custom-hook");
    }

    #[test]
    fn handles_claude_object_structure() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();

        let install_res = install_integration("claude", home);
        assert!(install_res.is_ok());
        assert!(get_integration_info("claude", home).integration_active);

        let uninstall_res = uninstall_integration("claude", home);
        assert!(uninstall_res.is_ok());
        assert!(!get_integration_info("claude", home).integration_active);
    }
}
