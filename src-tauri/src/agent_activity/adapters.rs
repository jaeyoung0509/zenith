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
        executables: &["opencode", "omp"],
        integration_available: false,
    },
];

/// Returns the canonical executable aliases for a known adapter. Keep Awake
/// typed rules consume this same allowlist instead of maintaining a second
/// process-signature table.
pub fn executable_aliases(adapter_id: &str) -> &'static [&'static str] {
    ADAPTERS
        .iter()
        .find(|adapter| adapter.id == adapter_id)
        .map_or(&[], |adapter| adapter.executables)
}

pub fn adapter_for_executable(path: &Path) -> Option<&'static AgentToolAdapter> {
    let home = crate::platform::NativePlatformPaths::new().home();
    adapter_for_executable_in_roots(path, home.as_deref(), &windows_install_roots())
}

fn adapter_for_executable_in_roots(
    path: &Path,
    home: Option<&Path>,
    windows_roots: &[std::path::PathBuf],
) -> Option<&'static AgentToolAdapter> {
    let path = InstallPath::parse(path)?;
    if !path.is_trusted(home, windows_roots) {
        return None;
    }
    ADAPTERS.iter().find(|adapter| {
        adapter
            .executables
            .iter()
            .any(|name| path.matches_executable(name))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathStyle {
    Unix,
    Windows,
}

/// A lexically validated install path. This does not grant process ownership:
/// callers still verify the process SID/UID, start time, and executable identity.
struct InstallPath {
    style: PathStyle,
    normalized: String,
}

impl InstallPath {
    fn parse(path: &Path) -> Option<Self> {
        let raw = path.to_str()?;
        let style = match raw.as_bytes() {
            [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic() => PathStyle::Windows,
            [b'\\', b'\\', ..] | [b'/', b'/', ..] => PathStyle::Windows,
            [b'/', ..] => PathStyle::Unix,
            _ => return None,
        };
        let normalized = match style {
            PathStyle::Unix => raw.to_owned(),
            PathStyle::Windows => {
                let normalized = raw.replace('\\', "/").to_ascii_lowercase();
                match normalized.strip_prefix("//?/") {
                    Some(rest) => match rest.strip_prefix("unc/") {
                        Some(unc) => format!("//{unc}"),
                        None if matches!(rest.as_bytes(),
                            [drive, b':', b'/', ..] if drive.is_ascii_alphabetic()) =>
                        {
                            rest.to_owned()
                        }
                        None => return None,
                    },
                    None if normalized.starts_with("//./") => return None,
                    None => normalized,
                }
            }
        };
        if normalized.split('/').any(|part| part == "..") {
            return None;
        }
        if style == PathStyle::Windows
            && (normalized
                .split('/')
                .any(|part| part.ends_with('.') || part.ends_with(' '))
                || normalized.rfind(':').is_some_and(|index| index != 1))
        {
            return None;
        }
        Some(Self {
            style,
            normalized: normalized.trim_end_matches('/').to_owned(),
        })
    }

    fn matches_executable(&self, expected: &str) -> bool {
        let Some(name) = self.normalized.rsplit('/').next() else {
            return false;
        };
        match self.style {
            PathStyle::Unix => name == expected,
            PathStyle::Windows => name.strip_suffix(".exe").unwrap_or(name) == expected,
        }
    }

    fn is_trusted(&self, home: Option<&Path>, windows_roots: &[std::path::PathBuf]) -> bool {
        let system_match = match self.style {
            PathStyle::Unix => UNIX_SYSTEM_ROOTS
                .iter()
                .any(|root| self.below(root).is_some_and(is_install_descendant)),
            PathStyle::Windows => windows_roots
                .iter()
                .filter_map(|root| Self::parse(root))
                .any(|root| {
                    self.below(&root.normalized)
                        .is_some_and(is_install_descendant)
                }),
        };
        if system_match {
            return true;
        }

        let Some(home) = home
            .and_then(Self::parse)
            .filter(|home| home.style == self.style)
        else {
            return false;
        };
        let Some(relative) = self.below(&home.normalized) else {
            return false;
        };
        let roots = match self.style {
            PathStyle::Unix => UNIX_USER_ROOTS,
            PathStyle::Windows => WINDOWS_USER_ROOTS,
        };
        roots.iter().any(|root| {
            relative
                .strip_prefix(root)
                .and_then(|tail| tail.strip_prefix('/'))
                .is_some_and(is_install_descendant)
        })
    }

    fn below<'a>(&'a self, root: &str) -> Option<&'a str> {
        self.normalized.strip_prefix(root)?.strip_prefix('/')
    }
}

fn is_install_descendant(relative: &str) -> bool {
    !relative.is_empty()
        && !relative.split('/').any(|part| {
            ["downloads", "desktop", "temp", "tmp"]
                .iter()
                .any(|denied| part.eq_ignore_ascii_case(denied))
        })
}

fn windows_install_roots() -> Vec<std::path::PathBuf> {
    #[cfg(windows)]
    {
        use crate::platform::PlatformPathsProvider;
        let paths = crate::platform::NativePlatformPaths::new();
        paths
            .application_roots()
            .into_iter()
            .chain(paths.program_data().map(|root| root.join("scoop/shims")))
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

const UNIX_SYSTEM_ROOTS: &[&str] = &[
    "/usr/bin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/nix/store",
    "/run/current-system/sw/bin",
    "/Applications",
];
const UNIX_USER_ROOTS: &[&str] = &[
    ".local/bin",
    ".local/share/mise/installs",
    ".cargo/bin",
    ".bun/bin",
    ".volta/bin",
    ".asdf/installs",
    ".nvm/versions",
    "Library/pnpm",
    "Library/Application Support",
];
const WINDOWS_USER_ROOTS: &[&str] = &[
    ".local/bin",
    ".cargo/bin",
    ".bun/bin",
    ".volta/bin",
    "appdata/local/programs",
    "appdata/roaming/npm",
    "appdata/local/npm",
    "appdata/local/pnpm",
    "appdata/local/yarn/bin",
    "scoop/shims",
    "scoop/apps",
];

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

    #[test]
    fn account_name_is_not_mistaken_for_a_temporary_install_directory() {
        let home = Some(Path::new("D:/Users/Temp"));
        assert!(adapter_for_executable_with_home(
            Path::new("D:/Users/Temp/.cargo/bin/codex.exe"),
            home
        )
        .is_some());
        assert!(adapter_for_executable_with_home(
            Path::new("D:/Users/Temp/.cargo/bin/tmp/codex.exe"),
            home
        )
        .is_none());
    }

    fn adapter_for_executable_with_home(
        path: &Path,
        home: Option<&Path>,
    ) -> Option<&'static AgentToolAdapter> {
        let roots = [
            "C:/Program Files",
            "C:/Program Files (x86)",
            "C:/ProgramData/scoop/shims",
        ]
        .map(std::path::PathBuf::from);
        adapter_for_executable_in_roots(path, home, &roots)
    }

    #[test]
    fn install_roots_reject_prefix_lookalikes_and_missing_home() {
        for path in [
            "/usr/local/bin-evil/codex",
            "/Applications-evil/codex",
            "C:/Program Files-evil/codex.exe",
            "D:/Users/홍 길동/.cargo/bin-evil/codex.exe",
            "D:/Users/홍 길동/AppData/Roaming/npm-evil/codex.exe",
            "/Program Files/codex.exe",
            "\\Program Files\\codex.exe",
        ] {
            assert!(
                adapter_for_executable_with_home(
                    Path::new(path),
                    Some(Path::new("D:/Users/홍 길동"))
                )
                .is_none(),
                "{path}"
            );
        }
        assert!(adapter_for_executable_with_home(
            Path::new("D:/Users/홍 길동/.cargo/bin/codex.exe"),
            None
        )
        .is_none());
    }

    #[test]
    fn windows_verbatim_and_case_variants_match_without_device_aliases() {
        let home = Some(Path::new("D:/Users/홍 길동"));
        assert_eq!(
            adapter_for_executable_with_home(
                Path::new(r"\\?\d:\USERS\홍 길동\.CARGO\BIN\CODEX.EXE"),
                home
            )
            .unwrap()
            .id,
            "codex"
        );
        for path in [
            r"\\.\D:\Users\홍 길동\.cargo\bin\codex.exe",
            r"D:\Users\홍 길동\.cargo\bin\codex.exe:stream",
        ] {
            assert!(adapter_for_executable_with_home(Path::new(path), home).is_none());
        }
    }

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
            adapter_for_executable_with_home(
                Path::new("C:\\Program Files\\Antigravity\\bin\\antigravity.exe"),
                None
            )
            .unwrap()
            .id,
            "antigravity"
        );

        assert_eq!(
            adapter_for_executable_with_home(
                Path::new("C:\\Program Files (x86)\\GitHub Copilot\\copilot.exe"),
                None
            )
            .unwrap()
            .id,
            "copilot"
        );

        assert_eq!(
            adapter_for_executable_with_home(
                Path::new("C:\\ProgramData\\scoop\\shims\\opencode.exe"),
                None
            )
            .unwrap()
            .id,
            "opencode"
        );
    }
}
