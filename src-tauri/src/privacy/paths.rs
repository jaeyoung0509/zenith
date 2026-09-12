//! Display-path masking shared by every IPC surface that must not reveal an
//! absolute location.
//!
//! The rule is: paths under the user home are rendered as `~/relative`, and
//! anything else is reduced to a basename with an out-of-home marker. The
//! absolute path is never returned as a fallback.

use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// Renders a path for display without leaking the absolute location.
///
/// The mask follows the described profile: a stated machine's own home is what
/// gets shortened, so a test or the doctor never masks with the host's.
pub fn display_path(path: &Path, environment: &crate::platform::PlatformEnvironment) -> String {
    display_path_with_home(path, environment.user_home().as_deref())
}

/// Pure form used by tests and by callers that already resolved the home path.
/// Comparison is done on normalized text so Windows-style separators mask
/// correctly on every host.
pub fn display_path_with_home(path: &Path, home: Option<&Path>) -> String {
    let path = crate::platform::NativePlatformPaths::normalize_verbatim_path(path);
    let home = home.map(crate::platform::NativePlatformPaths::normalize_verbatim_path);
    let home = home.as_deref();
    let normalized = normalize_separators(&path.to_string_lossy());
    let trimmed_path = normalized.trim_end_matches('/');
    if let Some(home) = home {
        let home_normalized = normalize_separators(&home.to_string_lossy());
        let home_trimmed = home_normalized.trim_end_matches('/');
        if !home_trimmed.is_empty() {
            if trimmed_path == home_trimmed {
                return "~".to_string();
            }
            let prefix = format!("{home_trimmed}/");
            if let Some(relative) = trimmed_path.strip_prefix(&prefix) {
                let relative = relative.trim_matches('/');
                if !relative.is_empty() {
                    return format!("~/{relative}");
                }
            }
        }
    }
    match trimmed_path.rsplit('/').find(|segment| !segment.is_empty()) {
        Some(name) => format!(".../{name}"),
        None => "<path>".to_string(),
    }
}

pub fn normalize_separators(value: &str) -> String {
    value.replace('\\', "/")
}

/// Masks absolute paths embedded in free-form text such as log lines.
///
/// Log lines describe the process that wrote them, so the native profile is the
/// one whose home is shortened here.
pub fn mask_paths_in_text(text: &str) -> String {
    let home = crate::platform::PlatformEnvironment::native().user_home();
    mask_paths_with_home(text, home.as_deref())
}

/// Pure form used by tests.
pub fn mask_paths_with_home(text: &str, home: Option<&Path>) -> String {
    let without_home = match home {
        Some(home) => mask_home_prefix(text, home),
        None => text.to_string(),
    };
    mask_absolute_paths(&without_home, home)
}

fn mask_home_prefix(text: &str, home: &Path) -> String {
    let home_str = home.to_string_lossy();
    if home_str.is_empty() {
        return text.to_string();
    }
    // Absorb an optional Windows verbatim prefix so `\\?\C:\Users\x\...` is
    // masked to `~/...` instead of leaving the prefix behind.
    let Ok(pattern) = Regex::new(&format!(
        r#"(?:\\\\\?\\)?(?:{})[\\/][^\s"'`,;:)\]]*"#,
        regex::escape(&home_str)
    )) else {
        return text.to_string();
    };
    pattern
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let matched = captures.get(0).map(|value| value.as_str()).unwrap_or("");
            let relative = matched
                .find(home_str.as_ref())
                .and_then(|index| matched.get(index + home_str.len()..))
                .unwrap_or("")
                .trim_start_matches(['\\', '/']);
            if relative.is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", normalize_separators(relative))
            }
        })
        .into_owned()
}

fn mask_absolute_paths(text: &str, home: Option<&Path>) -> String {
    static ABSOLUTE_PATH: LazyLock<Regex> = LazyLock::new(|| {
        // Covers POSIX (`/x`), Windows drive with either separator (`C:\x`,
        // `C:/x`), UNC (`\\server\share`), and verbatim (`\\?\...`) forms.
        Regex::new(r#"(^|[\s("'=,\[])((?:[A-Za-z]:[\\/]|\\\\|/)[^\s"'`,;)\]]*[\\/][^\s"'`,;)\]]+)"#)
            .expect("valid absolute path pattern")
    });
    ABSOLUTE_PATH
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let delimiter = captures.get(1).map(|value| value.as_str()).unwrap_or("");
            let path = captures.get(2).map(|value| value.as_str()).unwrap_or("");
            format!(
                "{delimiter}{}",
                display_path_with_home(Path::new(path), home)
            )
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_are_tildified_and_out_of_home_paths_keep_only_a_basename() {
        let home = Path::new("/Users/alice");
        assert_eq!(
            display_path_with_home(Path::new("/Users/alice/projects/repo"), Some(home)),
            "~/projects/repo"
        );
        assert_eq!(
            display_path_with_home(Path::new("/opt/src/repo"), Some(home)),
            ".../repo"
        );
        assert_eq!(
            display_path_with_home(Path::new("/Volumes/Work/repo"), Some(home)),
            ".../repo"
        );
    }

    #[test]
    fn unresolvable_home_never_falls_back_to_the_absolute_path() {
        assert_eq!(
            display_path_with_home(Path::new("/opt/src/repo"), None),
            ".../repo"
        );
        assert_eq!(display_path_with_home(Path::new("/"), None), "<path>");
    }

    #[test]
    fn windows_profiles_are_masked_with_forward_slashes() {
        assert_eq!(
            display_path_with_home(
                Path::new(r"C:\Users\alice\.claude\settings.json"),
                Some(Path::new(r"C:\Users\alice"))
            ),
            "~/.claude/settings.json"
        );
    }

    #[test]
    fn log_lines_lose_the_home_directory_and_user_name() {
        let masked = mask_paths_with_home(
            "failed to read /Users/alice/Library/Application Support/Zenith/settings.json",
            Some(Path::new("/Users/alice")),
        );
        assert!(!masked.contains("/Users/alice"));
        assert!(masked.contains("~/Library/Application Support/Zenith/settings.json"));
    }

    #[test]
    fn out_of_home_absolute_paths_are_reduced_but_urls_are_untouched() {
        let masked = mask_paths_with_home(
            "scan failed at /opt/src/private-repo/config.toml (see https://api.example.com/v1/key)",
            Some(Path::new("/Users/alice")),
        );
        assert!(masked.contains(".../config.toml"));
        assert!(!masked.contains("/opt/src/private-repo"));
        assert!(masked.contains("https://api.example.com/v1/key"));
    }

    #[test]
    fn windows_absolute_paths_outside_home_are_reduced() {
        let masked = mask_paths_with_home(
            r"target D:\Work\private\project\settings.json",
            Some(Path::new(r"C:\Users\alice")),
        );
        assert!(!masked.contains(r"D:\Work\private"));
        assert!(masked.contains("settings.json"));
    }

    #[test]
    fn windows_forward_slash_unc_and_verbatim_paths_are_masked() {
        let home = Path::new(r"C:\Users\alice");
        let cases = [
            (r"C:\Users\alice\secret-project\file.txt", true),
            ("C:/Users/alice/secret-project/file.txt", true),
            (r"\\server\share\secret-project\file.txt", false),
            (r"\\?\C:\Users\alice\secret-project\file.txt", true),
            (r"\\?\UNC\server\share\secret-project\file.txt", false),
        ];
        for (input, under_home) in cases {
            let masked = mask_paths_with_home(input, Some(home));
            assert!(masked.ends_with("file.txt"), "not reduced: {masked}");
            if under_home {
                assert!(!masked.contains("alice"), "user name leaked: {masked}");
                assert!(masked.starts_with("~/"), "expected home-relative: {masked}");
            } else {
                assert!(
                    !masked.contains("secret-project"),
                    "directory leaked: {masked}"
                );
                assert!(
                    masked.starts_with(".../"),
                    "expected out-of-home marker: {masked}"
                );
            }
        }
    }
}
