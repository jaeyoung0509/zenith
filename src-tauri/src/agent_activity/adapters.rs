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
        executables: &["agy", "antigravity"],
        integration_available: false,
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
        integration_available: false,
    },
    AgentToolAdapter {
        id: "cursor",
        display_name: "Cursor Agent CLI",
        executables: &["cursor-agent"],
        integration_available: false,
    },
    AgentToolAdapter {
        id: "grok",
        display_name: "Grok Build",
        executables: &["grok"],
        integration_available: false,
    },
    AgentToolAdapter {
        id: "copilot",
        display_name: "GitHub Copilot CLI",
        executables: &["copilot"],
        integration_available: false,
    },
    AgentToolAdapter {
        id: "opencode",
        display_name: "OpenCode",
        executables: &["opencode"],
        integration_available: false,
    },
];

pub fn adapter_for_executable(path: &Path) -> Option<&'static AgentToolAdapter> {
    adapter_for_executable_with_home(
        path,
        crate::platform::NativePlatformPaths::new().home().as_deref(),
    )
}

pub fn adapter_for_executable_with_home(
    path: &Path,
    home: Option<&Path>,
) -> Option<&'static AgentToolAdapter> {
    if !is_supported_install_path_internal(path, home) {
        return None;
    }
    let file_name = path_file_name(path)?;
    ADAPTERS
        .iter()
        .find(|adapter| {
            adapter
                .executables
                .iter()
                .any(|candidate| executable_name_matches(file_name, candidate))
        })
}

pub fn is_supported_install_path(path: &Path) -> bool {
    is_supported_install_path_internal(
        path,
        crate::platform::NativePlatformPaths::new().home().as_deref(),
    )
}

pub fn is_supported_install_path_with_home(path: &Path, home: Option<&Path>) -> bool {
    is_supported_install_path_internal(path, home)
}

fn path_file_name(path: &Path) -> Option<&str> {
    let s = path.to_str()?;
    let trimmed = s.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next()?;
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn executable_name_matches(file_name: &str, candidate: &str) -> bool {
    if file_name.eq_ignore_ascii_case(candidate) {
        return true;
    }
    if let Some((stem, ext)) = file_name.rsplit_once('.') {
        if ext.eq_ignore_ascii_case("exe") && stem.eq_ignore_ascii_case(candidate) {
            return true;
        }
    }
    false
}

fn is_absolute_path(path_str: &str) -> bool {
    if path_str.starts_with('/') || path_str.starts_with('\\') {
        return true;
    }
    if path_str.len() >= 3 {
        let bytes = path_str.as_bytes();
        if bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
        {
            return true;
        }
    }
    false
}

fn is_supported_install_path_internal(path: &Path, home: Option<&Path>) -> bool {
    let path_str = match path.to_str() {
        Some(s) => s,
        None => return false,
    };

    if !is_absolute_path(path_str) {
        return false;
    }

    // Reject parent traversal '..'
    if path_str.split(['/', '\\']).any(|part| part == "..") {
        return false;
    }

    let normalized = path_str.replace('\\', "/");
    let segments = normalized.split('/').collect::<Vec<_>>();

    // Reject unapproved user-content or temporary directories
    if segments.iter().any(|&s| {
        s.eq_ignore_ascii_case("downloads")
            || s.eq_ignore_ascii_case("desktop")
            || s.eq_ignore_ascii_case("temp")
            || s.eq_ignore_ascii_case("tmp")
    }) {
        return false;
    }

    // Unix system roots
    const UNIX_SYSTEM_ROOTS: &[&str] = &[
        "/usr/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/nix/store",
        "/run/current-system/sw/bin",
        "/Applications",
    ];
    if UNIX_SYSTEM_ROOTS.iter().any(|root| normalized.starts_with(root)) {
        return true;
    }

    // Windows system & Program Files roots
    if is_windows_system_or_program_path(&normalized) {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        let roots = crate::platform::NativePlatformPaths::new().application_roots();
        if roots.iter().any(|root| path.starts_with(root)) {
            return true;
        }
    }

    // User home roots
    let home = home
        .map(|p| p.to_path_buf())
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from));

    if let Some(home) = home {
        if let Some(home_str) = home.to_str() {
            let normalized_home = home_str.replace('\\', "/").trim_end_matches('/').to_string();
            if normalized.starts_with(&normalized_home) {
                let rel = &normalized[normalized_home.len()..];
                let rel = rel.strip_prefix('/').unwrap_or(rel);
                const USER_INSTALL_SUBDIRS: &[&str] = &[
                    ".local/bin",
                    ".local/share/mise/installs",
                    ".cargo/bin",
                    ".bun/bin",
                    ".volta/bin",
                    ".asdf/installs",
                    ".nvm/versions",
                    "Library/pnpm",
                    "Library/Application Support",
                    "AppData/Local/Programs",
                    "AppData/Roaming/npm",
                    "AppData/Local/npm",
                    "AppData/Local/pnpm",
                    "AppData/Local/yarn/bin",
                    "scoop/shims",
                    "scoop/apps",
                ];
                if USER_INSTALL_SUBDIRS.iter().any(|sub| rel.starts_with(sub)) {
                    return true;
                }
            }
        }
    }

    false
}

fn is_windows_system_or_program_path(normalized: &str) -> bool {
    let without_drive = if normalized.len() >= 3
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':'
        && normalized.as_bytes()[2] == b'/'
    {
        &normalized[2..]
    } else {
        normalized
    };

    const WINDOWS_SYSTEM_ROOTS: &[&str] = &[
        "/Program Files",
        "/Program Files (x86)",
        "/ProgramW6432",
        "/ProgramData/scoop/shims",
        "/Windows/System32",
    ];

    WINDOWS_SYSTEM_ROOTS
        .iter()
        .any(|root| without_drive.starts_with(root))
}

pub fn health(observed_ids: &std::collections::HashSet<&str>) -> Vec<AgentAdapterHealth> {
    health_with_integrations(observed_ids, &std::collections::HashSet::new())
}

pub fn health_with_integrations(
    observed_ids: &std::collections::HashSet<&str>,
    active_integrations: &std::collections::HashSet<&str>,
) -> Vec<AgentAdapterHealth> {
    ADAPTERS
        .iter()
        .map(|adapter| {
            let observed = observed_ids.contains(adapter.id);
            let integration_active = active_integrations.contains(adapter.id);
            let installed = observed
                || adapter
                    .executables
                    .iter()
                    .any(|executable| crate::tooling::resolve(executable).is_some());
            let state = if integration_active {
                AgentAdapterState::Connected
            } else if installed && adapter.integration_available {
                AgentAdapterState::IntegrationAvailable
            } else if installed {
                AgentAdapterState::ProcessOnly
            } else {
                AgentAdapterState::NotInstalled
            };
            let evidence = if integration_active {
                Some(AgentEvidence::VendorEvent)
            } else if observed {
                Some(AgentEvidence::ProcessObserved)
            } else {
                None
            };
            let message = match (state, observed) {
                (AgentAdapterState::Connected, _) => {
                    format!("{} · verified local integration active.", adapter.display_name)
                }
                (AgentAdapterState::IntegrationAvailable, true) => {
                    "Process observed · detailed local integration is available but not enabled.".to_string()
                }
                (AgentAdapterState::ProcessOnly, true) => {
                    if adapter.id == "gemini" {
                        "Process observed · legacy/enterprise fallback CLI. Transitioned to Antigravity CLI for individual accounts.".to_string()
                    } else {
                        "Process observed · detailed status unavailable.".to_string()
                    }
                }
                (AgentAdapterState::IntegrationAvailable, false) => {
                    "Installed · optional local integration is available; no active process observed.".to_string()
                }
                (AgentAdapterState::ProcessOnly, false) => {
                    if adapter.id == "gemini" {
                        "Installed · legacy/enterprise fallback CLI. Maintained for enterprise/API keys.".to_string()
                    } else {
                        "Installed · process-only observation; no active process observed.".to_string()
                    }
                }
                _ => "Not installed in a supported location.".to_string(),
            };
            AgentAdapterHealth {
                tool_id: adapter.id.to_string(),
                display_name: adapter.display_name.to_string(),
                state,
                evidence,
                message,
                installed_version: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
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

    #[test]
    fn test_windows_cli_recognition_korean_spaces() {
        let home = Path::new("D:\\Users\\홍 길동");

        assert_eq!(
            adapter_for_executable_with_home(
                Path::new("D:\\Users\\홍 길동\\.cargo\\bin\\codex.exe"),
                Some(home)
            )
            .unwrap()
            .id,
            "codex"
        );

        assert_eq!(
            adapter_for_executable_with_home(
                Path::new("D:\\Users\\홍 길동\\AppData\\Roaming\\npm\\claude.exe"),
                Some(home)
            )
            .unwrap()
            .id,
            "claude"
        );

        assert_eq!(
            adapter_for_executable_with_home(
                Path::new("D:\\Users\\홍 길동\\AppData\\Local\\Programs\\cursor\\cursor-agent.exe"),
                Some(home)
            )
            .unwrap()
            .id,
            "cursor"
        );

        assert_eq!(
            adapter_for_executable_with_home(
                Path::new("D:\\Users\\홍 길동\\scoop\\shims\\agy.exe"),
                Some(home)
            )
            .unwrap()
            .id,
            "antigravity"
        );
    }

    #[test]
    fn test_windows_cli_rejection_downloads_and_lookalikes() {
        let home = Path::new("D:\\Users\\홍 길동");

        // Reject downloads, desktop, and temp locations
        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\Downloads\\codex.exe"),
            Some(home)
        )
        .is_none());

        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\Desktop\\codex.exe"),
            Some(home)
        )
        .is_none());

        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\AppData\\Local\\Temp\\codex.exe"),
            Some(home)
        )
        .is_none());

        // Reject lookalikes and substrings
        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\.cargo\\bin\\codex-helper.exe"),
            Some(home)
        )
        .is_none());

        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\.cargo\\bin\\mycodex.exe"),
            Some(home)
        )
        .is_none());

        // Reject non-exe scripts / files
        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\.cargo\\bin\\codex.cmd"),
            Some(home)
        )
        .is_none());

        // Reject directory traversal
        assert!(adapter_for_executable_with_home(
            Path::new("D:\\Users\\홍 길동\\.cargo\\bin\\..\\Downloads\\codex.exe"),
            Some(home)
        )
        .is_none());

        // Reject relative paths
        assert!(adapter_for_executable_with_home(Path::new("codex.exe"), Some(home)).is_none());
    }

    #[test]
    fn test_windows_program_files_and_system_recognition() {
        assert_eq!(
            adapter_for_executable(Path::new(
                "C:\\Program Files\\Antigravity\\bin\\antigravity.exe"
            ))
            .unwrap()
            .id,
            "antigravity"
        );

        assert_eq!(
            adapter_for_executable(Path::new(
                "C:\\Program Files (x86)\\GitHub Copilot\\copilot.exe"
            ))
            .unwrap()
            .id,
            "copilot"
        );

        assert_eq!(
            adapter_for_executable(Path::new("C:\\ProgramData\\scoop\\shims\\opencode.exe"))
                .unwrap()
                .id,
            "opencode"
        );
    }
}
