//! Shared protected-process classification.
//!
//! Memory, development-port, and agent termination workflows must not drift.
//! This module owns the common deny-list covering terminals, shells, Zenith
//! itself, and platform system processes. Domain-specific protections (for
//! example databases or container runtimes) are layered on top by callers.

use std::path::Path;

/// Returns true when the process is a protected terminal, shell, Zenith
/// instance, or operating-system component that must never be signaled.
pub fn is_protected_process(
    process_name: &str,
    raw_command: Option<&str>,
    exe_path: Option<&Path>,
) -> bool {
    let name_lower = process_name.to_ascii_lowercase();
    let cmd_lower = raw_command.unwrap_or_default().to_ascii_lowercase();
    let exe_name = exe_path
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let matches_any = |targets: &[&str]| {
        targets.iter().any(|target| {
            name_lower == *target
                || name_lower == format!("{target}.exe")
                || cmd_lower == *target
                || cmd_lower == format!("{target}.exe")
                || exe_name == *target
                || exe_name == format!("{target}.exe")
                || name_lower.starts_with(&format!("{target}."))
        })
    };

    const TERMINALS: &[&str] = &[
        "terminal",
        "iterm2",
        "iterm",
        "alacritty",
        "kitty",
        "ghostty",
        "wezterm",
        "wezterm-gui",
        "warp",
        "hyper",
        "rio",
        "wt",
        "windowsterminal",
        "conhost",
        "openconsole",
        "mintty",
    ];
    if matches_any(TERMINALS) {
        return true;
    }

    const SHELLS: &[&str] = &[
        "sh",
        "bash",
        "zsh",
        "fish",
        "csh",
        "tcsh",
        "dash",
        "nu",
        "xonsh",
        "powershell",
        "pwsh",
        "cmd",
        "ssh",
        "sshd",
        "mosh-server",
        "mosh-client",
        "tmux",
        "screen",
        "login",
    ];
    if matches_any(SHELLS) {
        return true;
    }

    const ZENITH: &[&str] = &["zenith"];
    if matches_any(ZENITH) {
        return true;
    }

    const SYSTEM: &[&str] = &[
        "launchd",
        "systemd",
        "loginwindow",
        "securityagent",
        "coreauthd",
        "sudo",
        "su",
        "windowserver",
        "mds",
        "mdworker",
        "opendirectoryd",
        "syslogd",
        "notifyd",
        "configd",
        "diskarbitrationd",
        "distnoted",
        "cfprefsd",
        "rapportd",
        "controlcenter",
        "universalaccessd",
        "sharingd",
        "finder",
        "dock",
        "systemsettings",
        "svchost",
        "csrss",
        "services",
        "lsass",
        "smss",
        "wininit",
        "winlogon",
        "taskmgr",
        "explorer",
        "msmpeng",
        "securityhealthservice",
        "system",
        "ctfmon",
    ];
    if matches_any(SYSTEM) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn protects_common_terminal_applications() {
        for name in [
            "Terminal",
            "iTerm2",
            "Ghostty",
            "Alacritty",
            "kitty",
            "WezTerm",
            "Warp",
            "wt",
            "conhost",
        ] {
            assert!(
                is_protected_process(name, None, None),
                "expected {name} to be protected"
            );
        }
        assert!(is_protected_process("wezterm-gui", None, None));
    }

    #[test]
    fn protects_shells_and_zenith_itself() {
        for name in ["zsh", "bash", "fish", "sh", "powershell", "cmd", "tmux"] {
            assert!(
                is_protected_process(name, None, None),
                "expected {name} to be protected"
            );
        }
        assert!(is_protected_process("Zenith", None, None));
        assert!(is_protected_process("zenith.exe", None, None));
        assert!(is_protected_process(
            "helper",
            None,
            Some(Path::new("/Applications/Zenith.app/Contents/MacOS/Zenith"))
        ));
    }

    #[test]
    fn protects_platform_system_processes() {
        for name in [
            "launchd",
            "svchost",
            "csrss",
            "lsass",
            "explorer",
            "system",
            "ctfmon",
            "ctfmon.exe",
        ] {
            assert!(
                is_protected_process(name, None, None),
                "expected {name} to be protected"
            );
        }
    }

    #[test]
    fn allows_ordinary_user_applications() {
        for name in ["Cursor", "Google Chrome", "Code", "node", "claude"] {
            assert!(
                !is_protected_process(name, None, None),
                "expected {name} to be terminable"
            );
        }
    }
}
