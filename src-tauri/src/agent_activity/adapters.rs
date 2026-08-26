use crate::models::{AgentAdapterHealth, AgentAdapterState, AgentEvidence};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentToolAdapter {
    pub id: &'static str,
    pub display_name: &'static str,
    pub executables: &'static [&'static str],
    pub integration_available: bool,
}

pub const ADAPTERS: &[AgentToolAdapter] = &[
    AgentToolAdapter {
        id: "antigravity",
        display_name: "Antigravity",
        executables: &["agy"],
        integration_available: true,
    },
    AgentToolAdapter {
        id: "gemini",
        display_name: "Gemini CLI (legacy / enterprise)",
        executables: &["gemini"],
        integration_available: false,
    },
    AgentToolAdapter {
        id: "codex",
        display_name: "Codex CLI",
        executables: &["codex"],
        integration_available: false,
    },
    AgentToolAdapter {
        id: "claude",
        display_name: "Claude Code",
        executables: &["claude"],
        integration_available: true,
    },
    AgentToolAdapter {
        id: "cursor",
        display_name: "Cursor Agent CLI",
        executables: &["cursor-agent"],
        integration_available: true,
    },
    AgentToolAdapter {
        id: "grok",
        display_name: "Grok Build",
        executables: &["grok"],
        integration_available: true,
    },
    AgentToolAdapter {
        id: "copilot",
        display_name: "GitHub Copilot CLI",
        executables: &["copilot"],
        integration_available: true,
    },
    AgentToolAdapter {
        id: "opencode",
        display_name: "OpenCode",
        executables: &["opencode"],
        integration_available: true,
    },
];

pub fn adapter_for_executable(path: &Path) -> Option<&'static AgentToolAdapter> {
    if !is_supported_install_path(path) {
        return None;
    }
    let executable = path.file_name()?.to_str()?;
    ADAPTERS
        .iter()
        .find(|adapter| adapter.executables.contains(&executable))
}

fn is_supported_install_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }

    const SYSTEM_ROOTS: &[&str] = &[
        "/usr/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/nix/store",
        "/run/current-system/sw/bin",
        "/Applications",
    ];
    if SYSTEM_ROOTS.iter().any(|root| path.starts_with(root)) {
        return true;
    }

    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return false;
    };
    [
        ".local/bin",
        ".local/share/mise/installs",
        ".cargo/bin",
        ".bun/bin",
        ".volta/bin",
        ".asdf/installs",
        ".nvm/versions",
        "Library/pnpm",
        "Library/Application Support",
    ]
    .iter()
    .any(|root| path.starts_with(home.join(root)))
}

pub fn health(observed_ids: &std::collections::HashSet<&str>) -> Vec<AgentAdapterHealth> {
    ADAPTERS
        .iter()
        .map(|adapter| {
            let observed = observed_ids.contains(adapter.id);
            let installed = observed
                || adapter
                    .executables
                    .iter()
                    .any(|executable| crate::tooling::resolve(executable).is_some());
            let state = if installed && adapter.integration_available {
                AgentAdapterState::IntegrationAvailable
            } else if installed {
                AgentAdapterState::ProcessOnly
            } else {
                AgentAdapterState::NotInstalled
            };
            AgentAdapterHealth {
                tool_id: adapter.id.to_string(),
                display_name: adapter.display_name.to_string(),
                state,
                evidence: observed.then_some(AgentEvidence::ProcessObserved),
                message: match (state, observed) {
                    (AgentAdapterState::IntegrationAvailable, true) =>
                        "Process observed · detailed local integration is available but not enabled.".to_string(),
                    (AgentAdapterState::ProcessOnly, true) =>
                        "Process observed · detailed status unavailable.".to_string(),
                    (AgentAdapterState::IntegrationAvailable, false) =>
                        "Installed · optional local integration is available; no active process observed.".to_string(),
                    (AgentAdapterState::ProcessOnly, false) =>
                        "Installed · process-only observation; no active process observed.".to_string(),
                    _ => "Not installed in a supported location.".to_string(),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_executable_match_rejects_substrings_and_cursor_app() {
        assert_eq!(
            adapter_for_executable(Path::new("/opt/homebrew/bin/codex"))
                .unwrap()
                .id,
            "codex"
        );
        assert!(adapter_for_executable(Path::new("/tmp/codex-helper")).is_none());
        assert!(adapter_for_executable(Path::new("/tmp/codex")).is_none());
        assert!(adapter_for_executable(Path::new("codex")).is_none());
        assert!(adapter_for_executable(Path::new(
            "/Applications/Cursor.app/Contents/MacOS/Cursor"
        ))
        .is_none());
        assert_eq!(
            adapter_for_executable(Path::new("/usr/local/bin/cursor-agent"))
                .unwrap()
                .id,
            "cursor"
        );
    }
}
