use crate::models::*;
use crate::safety::Blacklist;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const MAX_FILES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 1_048_576;
const MAX_DEPTH: usize = 8;

pub fn inspect(
    projects: &std::collections::HashMap<String, PathBuf>,
    dismissed: &[String],
    now: u64,
) -> SafetySnapshot {
    let secret_patterns = [
        (
            "OpenAI-style API key",
            Regex::new(r"\bsk-[A-Za-z0-9_-]{16,}\b").unwrap(),
        ),
        (
            "GitHub token",
            Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").unwrap(),
        ),
        (
            "Private key material",
            Regex::new(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----").unwrap(),
        ),
        (
            "Credential assignment",
            Regex::new(r#"(?i)\b(?:api[_-]?key|token|password)\s*[:=]\s*['"]?[^\s'"]{12,}"#)
                .unwrap(),
        ),
    ];
    let dismissed = dismissed.iter().collect::<HashSet<_>>();
    let mut findings = Vec::new();
    let mut scanned = 0usize;
    let mut skipped = 0usize;
    let mut visited_entries = 0usize;
    let mut partial = false;
    for (project_id, root) in projects {
        if visited_entries >= MAX_FILES {
            partial = true;
            break;
        }
        if !is_eligible_safety_root(root) {
            partial = true;
            continue;
        }
        let root_device = device(root);
        let walker = WalkDir::new(root)
            .follow_links(false)
            .same_file_system(true)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(safe_entry);
        for entry in walker {
            visited_entries += 1;
            if visited_entries > MAX_FILES {
                partial = true;
                break;
            }
            let Ok(entry) = entry else {
                skipped += 1;
                partial = true;
                continue;
            };
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                skipped += 1;
                partial = true;
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES || device(entry.path()) != root_device {
                skipped += 1;
                partial = true;
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".into());
            if !is_scannable_file(entry.path(), &relative) {
                skipped += 1;
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                skipped += 1;
                partial = true;
                continue;
            };
            if bytes.contains(&0) {
                skipped += 1;
                continue;
            }
            let Ok(text) = String::from_utf8(bytes) else {
                skipped += 1;
                continue;
            };
            scanned += 1;
            for (line_index, line) in text.lines().enumerate() {
                for (category, pattern) in &secret_patterns {
                    if pattern.is_match(line) {
                        push_finding(&mut findings, project_id, SafetyFindingKind::SecretsExposure, FindingSeverity::Critical, category, "local_secret_detector", Some(relative.clone()), Some(line_index as u32 + 1), now, "Remove the exposed value, rotate it with the provider, and keep secrets outside the repository.", None, &dismissed);
                        break;
                    }
                }
            }
            if is_recognized_config(&relative) {
                inspect_config(project_id, &relative, &text, now, &dismissed, &mut findings);
            }
        }
    }
    findings.sort_by(|a, b| {
        severity_rank(b.severity)
            .cmp(&severity_rank(a.severity))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    SafetySnapshot {
        observed_at: now,
        quality: if partial {
            ObservationQuality::Partial
        } else {
            ObservationQuality::Fresh
        },
        findings,
        scanned_files: scanned as u32,
        skipped_files: skipped as u32,
        status_message: if partial {
            "Inspection reached a permission, size, file-count, depth, binary, or filesystem boundary.".into()
        } else {
            "Bounded local inspection completed.".into()
        },
    }
}

fn safe_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    !matches!(
        name.as_ref(),
        ".git" | "node_modules" | "target" | "dist" | "build" | ".venv" | "vendor"
    )
}
fn is_scannable_file(path: &Path, relative: &str) -> bool {
    if is_recognized_config(relative) {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        "rs" | "ts"
            | "js"
            | "svelte"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "rb"
            | "php"
            | "env"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "md"
            | "txt"
            | "sh"
    )
}
fn is_recognized_config(path: &str) -> bool {
    matches!(
        path,
        ".mcp.json"
            | "opencode.json"
            | "opencode.jsonc"
            | ".claude/settings.json"
            | ".claude/settings.local.json"
    )
}

fn inspect_config(
    project_id: &str,
    relative: &str,
    text: &str,
    now: u64,
    dismissed: &HashSet<&String>,
    findings: &mut Vec<SafetyFinding>,
) {
    let cleaned = if relative.ends_with(".jsonc") {
        strip_jsonc_comments(text)
    } else {
        text.to_string()
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&cleaned) else {
        return;
    };
    if let Some(servers) = value
        .get("mcpServers")
        .or_else(|| value.get("mcp"))
        .and_then(serde_json::Value::as_object)
    {
        for (name, config) in servers.iter().take(64) {
            let safe_name = sanitize_label(name);
            let transport = config
                .get("type")
                .and_then(serde_json::Value::as_str)
                .filter(|value| matches!(*value, "stdio" | "http" | "sse"))
                .map(str::to_string);
            let command_basename = config
                .get("command")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| Path::new(value).file_name())
                .map(|value| sanitize_label(&value.to_string_lossy()));
            let domain = config
                .get("url")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| url::Url::parse(value).ok())
                .and_then(|value| value.host_str().map(sanitize_label));
            let broad = config.get("env").is_some()
                || config.get("headers").is_some()
                || domain
                    .as_deref()
                    .is_some_and(|host| !matches!(host, "localhost" | "127.0.0.1" | "::1"));
            let evidence = NormalizedSafetyEvidence {
                server_name: Some(safe_name),
                scope: Some("project".into()),
                transport,
                permission_mode: None,
                sandbox_mode: None,
                command_basename,
                domain,
            };
            push_finding(findings, project_id, SafetyFindingKind::McpServers, if broad { FindingSeverity::Warning } else { FindingSeverity::Info }, if broad { "MCP server has remote or secret-bearing configuration" } else { "MCP server configured" }, if relative.starts_with(".claude") { "claude" } else { "opencode" }, Some(relative.into()), None, now, "Review the server scope and permissions in the owning tool. Zenith will not execute or rewrite this configuration.", Some(evidence), dismissed);
        }
    }
    let permission_text = [
        "permissions",
        "permission",
        "allowedTools",
        "allow",
        "sandbox",
    ]
    .iter()
    .filter_map(|key| value.get(*key))
    .map(|value| value.to_string())
    .collect::<String>()
    .to_ascii_lowercase();
    if permission_text.contains("bypass")
        || permission_text.contains("allowall")
        || permission_text.contains("dangerously")
        || permission_text.contains("/**")
    {
        push_finding(
            findings,
            project_id,
            SafetyFindingKind::ToolPermissions,
            FindingSeverity::Critical,
            "Overly broad tool permission",
            if relative.starts_with(".claude") {
                "claude"
            } else {
                "opencode"
            },
            Some(relative.into()),
            None,
            now,
            "Use the narrowest vendor permission and sandbox mode that supports this project.",
            None,
            dismissed,
        );
    }
    if permission_text.contains("/system")
        || permission_text.contains("/library")
        || permission_text.contains("../")
    {
        push_finding(findings, project_id, SafetyFindingKind::ProtectedPaths, FindingSeverity::Warning, "Permission may reach outside the project", "config_scope", Some(relative.into()), None, now, "Remove parent traversal and protected absolute paths from third-party tool permissions.", None, dismissed);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_finding(
    target: &mut Vec<SafetyFinding>,
    project_id: &str,
    kind: SafetyFindingKind,
    severity: FindingSeverity,
    evidence_type: &str,
    adapter: &str,
    path: Option<String>,
    line: Option<u32>,
    now: u64,
    remediation: &str,
    normalized: Option<NormalizedSafetyEvidence>,
    dismissed: &HashSet<&String>,
) {
    let mut hash = Sha256::new();
    hash.update(project_id);
    hash.update(format!("{kind:?}"));
    hash.update(evidence_type);
    if let Some(path) = &path {
        hash.update(path);
    }
    if let Some(line) = line {
        hash.update(line.to_le_bytes());
    }
    let id = format!("finding-{}", &format!("{:x}", hash.finalize())[..20]);
    target.push(SafetyFinding {
        dismissed: dismissed.contains(&id),
        id,
        project_id: project_id.into(),
        kind,
        severity,
        evidence_type: evidence_type.into(),
        adapter: adapter.into(),
        relative_path: path,
        line_start: line,
        line_end: line,
        observed_at: now,
        remediation: remediation.into(),
        normalized_evidence: normalized,
    });
}
fn sanitize_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
        .take(80)
        .collect()
}

fn is_eligible_safety_root(root: &Path) -> bool {
    if !root.is_dir() {
        return false;
    }
    let canonical = match root.canonicalize() {
        Ok(value) => Blacklist::normalize_path(&value),
        Err(_) => return false,
    };
    if canonical.parent().is_none() {
        return false;
    }
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if let Some(home) = home {
        let normalized_home = home
            .canonicalize()
            .map(|value| Blacklist::normalize_path(&value))
            .unwrap_or_else(|_| Blacklist::normalize_path(&home));
        if paths_equal_for_safety(&canonical, &normalized_home) {
            return false;
        }
    }
    let canonical_str = canonical.to_string_lossy().replace('\\', "/");
    let canonical_str = canonical_str.trim_end_matches('/');

    #[cfg(target_os = "windows")]
    if canonical_str.len() == 2
        && canonical_str.as_bytes()[0].is_ascii_alphabetic()
        && canonical_str.ends_with(':')
    {
        return false;
    }

    #[cfg(target_os = "windows")]
    if canonical_str.len() >= 3
        && canonical_str.as_bytes()[0].is_ascii_alphabetic()
        && canonical_str.as_bytes()[1] == b':'
        && [
            "/Users",
            "/Windows",
            "/Program Files",
            "/Program Files (x86)",
            "/ProgramData",
        ]
        .iter()
        .any(|denied| canonical_str[2..].eq_ignore_ascii_case(denied))
    {
        return false;
    }

    let broad_denylist = [
        "/",
        "/Users",
        "/home",
        "/System",
        "/Library",
        "/Applications",
        "/private",
        "/usr",
        "/bin",
        "/sbin",
        "/etc",
        "/var",
        "/Volumes",
        "/tmp",
        "/opt",
    ];
    for denied in broad_denylist {
        #[cfg(target_os = "windows")]
        let matches = canonical_str.eq_ignore_ascii_case(denied);
        #[cfg(not(target_os = "windows"))]
        let matches = canonical_str == denied;
        if matches {
            return false;
        }
    }
    if canonical.components().count() <= 2 {
        return false;
    }
    true
}

fn paths_equal_for_safety(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

fn strip_jsonc_comments(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut in_single_comment = false;
    let mut in_block_comment = false;
    let chars: Vec<char> = value.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        let next_ch = if i + 1 < len {
            Some(chars[i + 1])
        } else {
            None
        };

        if in_single_comment {
            if ch == '\n' {
                in_single_comment = false;
                result.push('\n');
            }
            i += 1;
            continue;
        }

        if in_block_comment {
            if ch == '*' && next_ch == Some('/') {
                in_block_comment = false;
                i += 2;
            } else {
                if ch == '\n' {
                    result.push('\n');
                }
                i += 1;
            }
            continue;
        }

        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(ch);
            i += 1;
        } else if ch == '/' && next_ch == Some('/') {
            in_single_comment = true;
            i += 2;
        } else if ch == '/' && next_ch == Some('*') {
            in_block_comment = true;
            i += 2;
        } else {
            result.push(ch);
            i += 1;
        }
    }

    result
}
fn severity_rank(value: FindingSeverity) -> u8 {
    match value {
        FindingSeverity::Info => 0,
        FindingSeverity::Warning => 1,
        FindingSeverity::Critical => 2,
    }
}
#[cfg(unix)]
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    path.metadata().ok().map(|metadata| metadata.dev())
}
#[cfg(not(unix))]
fn device(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_secret_category_and_line_without_value() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("main.ts"),
            "const token = 'sk-abcdefghijklmnop1234';\n",
        )
        .unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&roots, &[], 10);
        assert_eq!(result.findings.len(), 1);
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("abcdefghijklmnop1234"));
        assert_eq!(result.findings[0].line_start, Some(1));
    }
    #[test]
    fn config_parser_never_returns_args_headers_or_env_values() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".mcp.json"), r#"{"mcpServers":{"demo":{"type":"stdio","command":"/usr/bin/node","args":["SECRET"],"env":{"TOKEN":"hidden"},"headers":{"Authorization":"hidden"}}}}"#).unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&roots, &[], 10);
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("SECRET"));
        assert!(!json.contains("hidden"));
        assert!(json.contains("node"));
    }
    #[test]
    fn symlinked_files_are_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "sk-abcdefghijklmnop1234").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), temp.path().join("linked.txt")).unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        assert!(inspect(&roots, &[], 10).findings.is_empty());
    }

    #[test]
    fn safety_scan_traversal_hard_caps_at_max_files_even_for_skipped_files() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..2050 {
            std::fs::write(
                temp.path().join(format!("img_{}.png", i)),
                b"fake image data",
            )
            .unwrap();
        }
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&roots, &[], 10);
        assert_eq!(result.quality, ObservationQuality::Partial);
        assert!(result.status_message.contains("boundary"));
    }

    #[test]
    fn strip_jsonc_preserves_urls_and_strips_comments() {
        let jsonc_input = r#"{
            // Line comment with // multiple slashes
            "mcpServers": {
                /* Block comment */
                "remote": {
                    "url": "https://example.com/mcp?foo=//bar"
                }
            }
        }"#;
        let stripped = strip_jsonc_comments(jsonc_input);
        let parsed: serde_json::Value =
            serde_json::from_str(&stripped).expect("valid JSON after comment stripping");
        assert_eq!(
            parsed["mcpServers"]["remote"]["url"].as_str(),
            Some("https://example.com/mcp?foo=//bar")
        );
    }

    #[test]
    fn broad_roots_such_as_home_or_root_are_rejected_from_automatic_scan() {
        assert!(!is_eligible_safety_root(Path::new("/")));
        #[cfg(unix)]
        {
            assert!(!is_eligible_safety_root(Path::new("/Users")));
            assert!(!is_eligible_safety_root(Path::new("/System")));
        }
        #[cfg(target_os = "windows")]
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from);
        #[cfg(not(target_os = "windows"))]
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        if let Some(home) = home {
            assert!(!is_eligible_safety_root(&home));

            #[cfg(target_os = "windows")]
            {
                let users_root = home.parent().expect("user profile has a parent");
                assert!(!is_eligible_safety_root(users_root));
                let drive_root = home.ancestors().last().expect("user profile has a root");
                assert!(!is_eligible_safety_root(drive_root));
            }
        }
    }
}
