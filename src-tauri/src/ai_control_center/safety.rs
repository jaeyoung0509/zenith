use crate::models::*;
use crate::platform::description::PlatformEnvironment;
use crate::platform::path_algebra;
use crate::privacy::secrets;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const MAX_ENTRIES_PER_ROOT: usize = 2_000;
const MAX_FILE_BYTES: u64 = 1_048_576;
const MAX_DEPTH: usize = 8;
const MAX_FINDINGS_PER_FILE: usize = 10;

/// Decodes a selected file, accepting an explicit UTF-8/UTF-16 BOM. A file
/// that cannot be decoded returns `None` so the caller can mark the result
/// partial instead of reporting a clean inspection.
fn decode_scannable_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(rest).ok().map(str::to_owned);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, u16::from_be_bytes);
    }
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

fn decode_utf16(bytes: &[u8], to_unit: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes.as_chunks::<2>().0.iter().map(|pair| to_unit(*pair));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

pub fn inspect(
    environment: &PlatformEnvironment,
    projects: &std::collections::HashMap<String, PathBuf>,
    dismissed: &[String],
    now: u64,
) -> SafetySnapshot {
    let dismissed = dismissed.iter().collect::<HashSet<_>>();
    // HashMap order is non-deterministic; sort so one repository cannot decide
    // which other repositories are reached and so outputs are reproducible.
    let mut ordered = projects.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(b.0)));

    let mut findings = Vec::new();
    let mut scanned = 0usize;
    let mut skipped = 0usize;
    let mut partial = false;
    let mut inspected_roots = Vec::new();
    let mut unreached_roots = Vec::new();
    let mut boundary_reasons = Vec::new();

    for (project_id, root) in ordered.iter() {
        if !is_eligible_safety_root(environment, root) {
            partial = true;
            unreached_roots.push(root_label(root));
            boundary_reasons.push(format!(
                "{} is outside the eligible project scope",
                root_label(root)
            ));
            continue;
        }
        inspected_roots.push(root_label(root));
        let root_device = device(root);
        let mut visited_entries = 0usize;
        let walker = WalkDir::new(root)
            .follow_links(false)
            .same_file_system(true)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(safe_entry);
        for entry in walker {
            visited_entries += 1;
            if visited_entries > MAX_ENTRIES_PER_ROOT {
                // The budget is per root: stop walking this repository and
                // continue with the next one so one large tree cannot hide
                // findings in the remaining projects.
                partial = true;
                boundary_reasons.push(format!(
                    "the {MAX_ENTRIES_PER_ROOT}-entry budget was exhausted in {}",
                    root_label(root)
                ));
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
                boundary_reasons.push(format!(
                    "{} was skipped in {} by a size or filesystem boundary",
                    file_label(entry.path()),
                    root_label(root)
                ));
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|path| crate::privacy::paths::normalize_separators(&path.to_string_lossy()))
                .unwrap_or_else(|| "unknown".into());
            if !is_scannable_file(entry.path(), &relative) {
                skipped += 1;
                if looks_credential_bearing(&relative) {
                    partial = true;
                    boundary_reasons.push(format!(
                        "credential-bearing file {relative} was excluded by the selection gate"
                    ));
                }
                continue;
            }
            let Ok(bytes) = std::fs::read(entry.path()) else {
                skipped += 1;
                partial = true;
                boundary_reasons.push(format!("{relative} could not be read"));
                continue;
            };
            let Some(text) = decode_scannable_text(&bytes) else {
                // A selected credential file that cannot be decoded is a real
                // boundary: reporting Fresh would repeat the false-confidence
                // failure this scan exists to remove.
                skipped += 1;
                partial = true;
                boundary_reasons.push(format!("{relative} could not be decoded for inspection"));
                continue;
            };
            scanned += 1;
            // Report every credential shape in the file, bounded per file so a
            // generated file cannot flood the result.
            let mut file_findings = 0usize;
            for (line_index, line) in text.lines().enumerate() {
                if let Some(category) = secrets::match_category(line) {
                    push_finding(&mut findings, project_id, SafetyFindingKind::SecretsExposure, FindingSeverity::Critical, category, "local_secret_detector", Some(relative.clone()), Some(line_index as u32 + 1), now, "Remove the exposed value, rotate it with the provider, and keep secrets outside the repository.", None, &dismissed);
                    file_findings += 1;
                    if file_findings >= MAX_FINDINGS_PER_FILE {
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
    let status_message = if partial {
        let mut message = "Inspection did not complete; a boundary was reached.".to_string();
        if !unreached_roots.is_empty() {
            message.push_str(&format!(
                " Roots not inspected: {}.",
                unreached_roots.join(", ")
            ));
        }
        // Keep distinct reasons so an exhausted budget and an oversized file
        // are not collapsed into a single generic message.
        let mut reasons = Vec::new();
        for reason in &boundary_reasons {
            if !reasons.contains(reason) {
                reasons.push(reason.clone());
            }
            if reasons.len() == 3 {
                break;
            }
        }
        if !reasons.is_empty() {
            message.push_str(&format!(" {}", reasons.join("; ")));
        }
        message
    } else {
        "Bounded local inspection completed.".into()
    };
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
        inspected_roots,
        unreached_roots,
        status_message,
    }
}

fn root_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "<root>".into())
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "<file>".into())
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
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if is_scannable_name(&name) {
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
            | "pem"
            | "key"
            | "p12"
            | "pfx"
    )
}

/// Dotfiles such as `.env` have no extension and must be selected by name.
fn is_scannable_name(name: &str) -> bool {
    matches!(
        name,
        ".env"
            | ".npmrc"
            | ".netrc"
            | "credentials"
            | "id_rsa"
            | "id_ed25519"
            | "id_ecdsa"
            | "id_dsa"
    ) || name.starts_with(".env.")
        || name.starts_with("credentials.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
}

/// Names that are expected to hold credentials even though no source extension
/// matches. Excluding one keeps the result partial so completion is never
/// reported while a credential file was skipped.
fn looks_credential_bearing(relative: &str) -> bool {
    let name = relative
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(relative)
        .to_ascii_lowercase();
    is_scannable_name(&name)
        || name.contains("secret")
        || name.contains("token")
        || name.contains("credential")
        || name.contains("password")
        || name.contains("apikey")
        || name.contains("api_key")
        || name.ends_with(".pem")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
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

/// Whether a project root may be walked by the automatic safety inspection.
///
/// The rules are the environment's own path rules, not the host's: a Windows
/// drive that is not `C:`, a UNC profile, and a POSIX profile are all decided
/// by the same flavor-parameterized algebra, so the refusal is provable on any
/// runner. The profile comes from the environment, never from the host.
fn is_eligible_safety_root(environment: &PlatformEnvironment, root: &Path) -> bool {
    if !root.is_dir() {
        return false;
    }
    let flavor = environment.flavor();
    let Ok(canonical) = root.canonicalize() else {
        return false;
    };
    let canonical_text = path_algebra::normalize(&canonical.to_string_lossy(), flavor);
    let home_text = environment.user_home().map(|home| {
        home.canonicalize()
            .map(|value| path_algebra::normalize(&value.to_string_lossy(), flavor))
            .unwrap_or_else(|_| path_algebra::normalize(&home.to_string_lossy(), flavor))
    });
    !safety_root_is_ineligible(flavor, &canonical_text, home_text.as_deref())
}

/// True when a canonical location is too broad to scan as one project.
///
/// Pure and flavor-parameterized so a Windows drive that is not `C:`, a UNC
/// profile, and a POSIX profile are all decided by the same rules on any
/// runner.
fn safety_root_is_ineligible(
    flavor: path_algebra::PathFlavor,
    canonical_text: &str,
    home_text: Option<&str>,
) -> bool {
    if path_algebra::is_root(canonical_text, flavor)
        || path_algebra::is_unsupported_namespace(canonical_text, flavor)
    {
        return true;
    }
    // An 8.3 alias cannot be resolved without the volume, so a root whose
    // component might be one is refused rather than assumed harmless.
    if path_algebra::contains_short_name(canonical_text, flavor) {
        return true;
    }
    // A system directory (and everything below it on Windows: Windows,
    // Program Files, ProgramData, the users container) is never a project.
    // POSIX system prefixes are owned by the broad-root rule below, which
    // deliberately still allows a project inside e.g. `/private/var`.
    let protected = path_algebra::protected_root(canonical_text, flavor);
    if protected.is_some_and(|rule| rule != path_algebra::ProtectedRoot::PosixSystemPrefix) {
        return true;
    }
    // A POSIX location directly under the root (`/Users`, `/home`, `/tmp`,
    // `/Volumes`, `/opt`) is too broad to be one project's root.
    if !flavor.is_windows() && components_below_root(canonical_text, flavor).len() <= 1 {
        return true;
    }
    match home_text {
        Some(home_text) => path_algebra::equal(canonical_text, home_text, flavor),
        None => false,
    }
}

/// Path components below the flavor's root prefix: `C:\a\b` and
/// `\\server\share\a\b` yield `["a", "b"]`, `/a/b` yields `["a", "b"]`.
fn components_below_root(path: &str, flavor: path_algebra::PathFlavor) -> Vec<String> {
    let canonical =
        path_algebra::canonical_separators(&path_algebra::strip_verbatim(path, flavor), flavor);
    let remainder = if flavor.is_windows() {
        if canonical.starts_with(r"\\") {
            canonical
                .trim_start_matches('\\')
                .splitn(3, '\\')
                .nth(2)
                .unwrap_or_default()
        } else if canonical.len() >= 2 && canonical.as_bytes()[1] == b':' {
            canonical[2..].trim_start_matches('\\')
        } else {
            canonical.as_str()
        }
    } else {
        canonical.trim_start_matches('/')
    };
    remainder
        .split(flavor.separator())
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect()
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
    use crate::platform::path_algebra::PathFlavor;

    /// An environment that states no profile, so the host's home never decides
    /// whether a temporary project root is eligible.
    fn simulated() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    #[test]
    fn reports_secret_category_and_line_without_value() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("main.ts"),
            "const token = 'sk-abcdefghijklmnop1234';\n",
        )
        .unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&simulated(), &roots, &[], 10);
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
        let result = inspect(&simulated(), &roots, &[], 10);
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("SECRET"));
        assert!(!json.contains("hidden"));
        assert!(json.contains("node"));
    }
    #[test]
    fn symlinked_files_are_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "sk-abcdefghijklmnop1234").unwrap();
        // The ordinary file inside the scanned root must always be reported, so
        // this assertion is meaningful on every platform, not only where a
        // symlink can be created.
        std::fs::write(temp.path().join("ordinary.txt"), "sk-abcdefghijklmnop1234").unwrap();
        let outside_file = outside.path().join("secret.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_file, temp.path().join("linked.txt")).unwrap();
        #[cfg(windows)]
        {
            // Creating a file symlink may require Developer Mode; the assertion
            // below still runs when it cannot be created.
            let _ =
                std::os::windows::fs::symlink_file(&outside_file, temp.path().join("linked.txt"));
        }

        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&simulated(), &roots, &[], 10);
        assert_eq!(
            result.findings.len(),
            1,
            "only the file inside the root may be reported, never a link target"
        );

        // A root that does not exist is not reported as a clean inspection.
        let missing = std::collections::HashMap::from([(
            "missing".into(),
            temp.path().join("does-not-exist"),
        )]);
        let result = inspect(&simulated(), &missing, &[], 10);
        assert!(result.findings.is_empty());
        assert_eq!(result.quality, ObservationQuality::Partial);
        assert_eq!(result.scanned_files, 0);
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
        let result = inspect(&simulated(), &roots, &[], 10);
        assert_eq!(result.quality, ObservationQuality::Partial);
        assert!(result.status_message.contains("boundary"));
        assert!(result.status_message.contains("budget"));
    }

    #[test]
    fn dotfiles_without_extensions_are_scanned_by_name() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".env"),
            format!(
                "AWS_SECRET_ACCESS_KEY={}\n",
                concat!("wJalrXUtnFEMI", "/K7MDENG/", "bPxRfiCYEXAMPLEKEY")
            ),
        )
        .unwrap();
        std::fs::write(
            temp.path().join(".env.production"),
            "API_KEY=abcdef123456\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join(".npmrc"),
            "//registry.npmjs.org/:_authToken=npm_abcdefghijklmnopqrstuvwx\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("id_rsa"),
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
        )
        .unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&simulated(), &roots, &[], 10);
        assert!(
            result.scanned_files >= 4,
            "expected dotfiles to be scanned, got {}",
            result.scanned_files
        );
        assert!(result.findings.len() >= 4);
        assert_eq!(result.quality, ObservationQuality::Fresh);
    }

    #[test]
    fn utf16_encoded_credential_files_are_decoded_and_scanned() {
        let temp = tempfile::tempdir().unwrap();
        let mut bytes = vec![0xFF, 0xFE];
        let env_line = format!(
            "AWS_SECRET_ACCESS_KEY={}\n",
            concat!("wJalrXUtnFEMI", "/K7MDENG/", "bPxRfiCYEXAMPLEKEY")
        );
        for unit in env_line.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(temp.path().join(".env"), bytes).unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&simulated(), &roots, &[], 10);
        assert!(!result.findings.is_empty(), "UTF-16 .env was not inspected");
        assert_eq!(result.quality, ObservationQuality::Fresh);
    }

    #[test]
    fn undecodable_credential_file_marks_the_result_partial() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".env"),
            b"AWS_SECRET_ACCESS_KEY=\xFF\x00\x00binary",
        )
        .unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&simulated(), &roots, &[], 10);
        assert_eq!(result.quality, ObservationQuality::Partial);
        assert!(result.status_message.contains("could not be decoded"));
    }

    #[test]
    fn every_credential_line_in_a_file_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let content = format!(
            "const a = '{}';\nconst b = '{}';\nconst c = '{}';\n",
            concat!("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
            concat!("sk-", "abcdefghijklmnop1234"),
            concat!("AIzaSy", "abcdefghijklmnopqrstuvwxyz0123456"),
        );
        std::fs::write(temp.path().join("main.ts"), content).unwrap();
        let roots = std::collections::HashMap::from([("p".into(), temp.path().into())]);
        let result = inspect(&simulated(), &roots, &[], 10);
        assert_eq!(
            result.findings.len(),
            3,
            "every distinct credential line must be reported"
        );
    }

    #[test]
    fn credential_bearing_names_are_recognized_for_the_partial_gate() {
        for name in [
            ".env",
            ".env.local",
            ".npmrc",
            ".netrc",
            "credentials.json",
            "id_rsa",
            "server.pem",
        ] {
            assert!(looks_credential_bearing(name), "not flagged: {name}");
        }
        assert!(!looks_credential_bearing("README.md"));
    }

    #[test]
    fn the_entry_budget_is_per_root_and_unreached_roots_are_reported() {
        let base = tempfile::tempdir().unwrap();
        let heavy = base.path().join("a-heavy");
        let secret_root = base.path().join("z-secret");
        std::fs::create_dir_all(&heavy).unwrap();
        std::fs::create_dir_all(&secret_root).unwrap();
        for index in 0..2_050 {
            std::fs::write(heavy.join(format!("img_{index}.png")), b"fake image data").unwrap();
        }
        std::fs::write(
            secret_root.join("main.ts"),
            "const key = 'sk-abcdefghijklmnop1234';\n",
        )
        .unwrap();

        let roots = std::collections::HashMap::from([
            ("heavy".to_string(), heavy.clone()),
            ("secret".to_string(), secret_root.clone()),
        ]);
        let result = inspect(&simulated(), &roots, &[], 10);
        assert_eq!(result.quality, ObservationQuality::Partial);
        assert!(result.status_message.contains("budget"));
        // The heavy root is truncated, but the later root is still scanned: a
        // per-root budget must not let one repository hide another's findings.
        assert!(result.inspected_roots.iter().any(|root| root == "a-heavy"));
        assert!(result.inspected_roots.iter().any(|root| root == "z-secret"));
        assert!(
            !result.unreached_roots.iter().any(|root| root == "z-secret"),
            "a later root must not be reported unreached for another root's budget"
        );
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.project_id == "secret"));
    }

    #[test]
    fn windows_style_relative_paths_are_recognized_after_normalization() {
        let normalized = crate::privacy::paths::normalize_separators(r".claude\settings.json");
        assert_eq!(normalized, ".claude/settings.json");
        assert!(is_recognized_config(&normalized));
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
        let environment = simulated();
        let flavor = environment.flavor();

        assert!(safety_root_is_ineligible(flavor, "/", None));
        #[cfg(unix)]
        {
            for denied in [
                "/Users",
                "/System",
                "/Volumes",
                "/tmp",
                "/opt",
                "/Applications",
            ] {
                assert!(
                    safety_root_is_ineligible(flavor, denied, None),
                    "{denied} must not be an eligible scan root"
                );
            }
            assert!(!safety_root_is_ineligible(
                flavor,
                "/Users/me/dev/project",
                None
            ));
            // Temporary trees are legitimate project locations.
            assert!(!safety_root_is_ineligible(
                flavor,
                "/private/var/folders/t/abc/T/project",
                None
            ));
        }

        // The stated profile is never an eligible root, and neither is a
        // stated profile that happens to live outside the host's home.
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("profile");
        std::fs::create_dir_all(home.join("dev/project")).unwrap();
        let stated = PlatformEnvironment::simulated(PathFlavor::current()).with_home(&home);
        assert!(!is_eligible_safety_root(&stated, &home));
        assert!(is_eligible_safety_root(&stated, &home.join("dev/project")));
    }

    #[test]
    fn windows_flavor_rules_accept_a_non_system_drive_and_refuse_system_tails() {
        let flavor = PathFlavor::Windows;
        let home = r"C:\Users\me";

        assert!(!safety_root_is_ineligible(
            flavor,
            r"D:\dev\project",
            Some(home)
        ));
        assert!(!safety_root_is_ineligible(
            flavor,
            r"C:\Users\me\dev\project",
            Some(home)
        ));
        for denied in [
            r"C:\",
            r"D:\",
            r"C:\Users",
            r"D:\Windows\System32",
            r"D:\Program Files\App",
            r"D:\ProgramData",
            home,
        ] {
            assert!(
                safety_root_is_ineligible(flavor, denied, Some(home)),
                "{denied} must not be an eligible scan root"
            );
        }

        // A UNC profile is refused as the profile itself, but a project below
        // it is accepted.
        let unc_home = r"\\fileserver\profiles\me";
        assert!(safety_root_is_ineligible(flavor, unc_home, Some(unc_home)));
        assert!(!safety_root_is_ineligible(
            flavor,
            r"\\fileserver\profiles\me\dev",
            Some(unc_home)
        ));
    }
}
