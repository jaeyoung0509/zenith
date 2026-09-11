use crate::models::{AgentIntegrationInfo, AgentIntegrationResult};
use std::path::Path;

const ZENITH_HOOK_ID: &str = "zenith-agent-activity";

pub fn get_integration_info(tool_id: &str, home_dir: &Path) -> AgentIntegrationInfo {
    let tool = tool_integration(tool_id);

    if !tool.supported {
        return AgentIntegrationInfo {
            tool_id: tool_id.to_string(),
            display_name: tool.display_name.to_string(),
            supported: false,
            installed: false,
            integration_active: false,
            config_path: None,
            description: tool.description.to_string(),
        };
    }

    let config_path = home_dir.join(tool.config_rel_path);
    let installed = config_path.exists();
    let integration_active = if installed {
        is_hook_present(&config_path)
    } else {
        false
    };

    AgentIntegrationInfo {
        tool_id: tool_id.to_string(),
        display_name: tool.display_name.to_string(),
        supported: true,
        installed,
        integration_active,
        // Display paths must never leak the absolute home location across IPC.
        config_path: Some(crate::privacy::paths::display_path(&config_path)),
        description: tool.description.to_string(),
    }
}

struct ToolIntegration {
    display_name: &'static str,
    supported: bool,
    config_rel_path: &'static str,
    description: &'static str,
}

fn tool_integration(tool_id: &str) -> ToolIntegration {
    match tool_id {
        "antigravity" => ToolIntegration {
            display_name: "Antigravity",
            supported: true,
            config_rel_path: ".gemini/antigravity/hooks.json",
            description:
                "Process-only observation. A verified Zenith event bridge is not available yet.",
        },
        "claude" => ToolIntegration {
            display_name: "Claude Code",
            supported: true,
            config_rel_path: ".claude/settings.json",
            description:
                "Process-only observation. A verified Zenith event bridge is not available yet.",
        },
        "cursor" => ToolIntegration {
            display_name: "Cursor Agent CLI",
            supported: true,
            config_rel_path: ".cursor/hooks.json",
            description:
                "Process-only observation. A verified Zenith event bridge is not available yet.",
        },
        "grok" => ToolIntegration {
            display_name: "Grok Build",
            supported: true,
            config_rel_path: ".grok/hooks.json",
            description:
                "Process-only observation. A verified Zenith event bridge is not available yet.",
        },
        "copilot" => ToolIntegration {
            display_name: "GitHub Copilot CLI",
            supported: true,
            config_rel_path: ".copilot/hooks.json",
            description:
                "Process-only observation. A verified Zenith event bridge is not available yet.",
        },
        "gemini" => ToolIntegration {
            display_name: "Gemini CLI (legacy / enterprise)",
            supported: false,
            config_rel_path: "",
            description:
                "Process-only observation. Individual accounts transitioned to Antigravity CLI.",
        },
        "codex" => ToolIntegration {
            display_name: "Codex CLI",
            supported: false,
            config_rel_path: "",
            description: "Process-only observation for unmanaged TUI.",
        },
        "opencode" => ToolIntegration {
            display_name: "OpenCode",
            supported: false,
            config_rel_path: "",
            description: "Process-only observation.",
        },
        _ => ToolIntegration {
            display_name: "Unknown tool",
            supported: false,
            config_rel_path: "",
            description: "Unsupported tool.",
        },
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
    let tool = tool_integration(tool_id);
    if !tool.supported || tool.config_rel_path.is_empty() {
        return Err("Configuration path not determined.".to_string());
    }
    // Rebuild the real path from the tool definition; `get_integration_info`
    // returns a masked display path that must never be used for file access.
    let config_path = home_dir.join(tool.config_rel_path);

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
        message: format!("Local integration for {} removed.", tool.display_name),
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
    // A third-party settings file may hold credentials, so refuse to rewrite
    // when its current permissions cannot be read and preserved instead of
    // replacing it with a wider mode.
    let permissions = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read config permissions: {e}"))?
        .permissions();
    let temp_path = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::write(&temp_path, serialized) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to write temporary config: {error}"));
    }
    if let Err(error) = std::fs::set_permissions(&temp_path, permissions) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to preserve config permissions: {error}"));
    }
    // `atomic_replace` uses ReplaceFileW on Windows, which keeps the replaced
    // file's ACL, and a plain rename on Unix where the temp mode is already set.
    if let Err(error) = crate::platform::file_ops::atomic_replace(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to atomically replace config file: {error}"));
    }
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

    #[test]
    fn integration_config_path_is_masked_before_ipc() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let info = get_integration_info("claude", &home);
        let display = info.config_path.expect("supported tool has a display path");
        assert!(!display.starts_with('/'), "absolute path leaked: {display}");
        assert!(display.ends_with("settings.json"));
    }

    #[cfg(unix)]
    #[test]
    fn rewriting_a_third_party_config_preserves_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let path = home.join(".claude/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"hooks":{}}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        uninstall_integration("claude", home).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
