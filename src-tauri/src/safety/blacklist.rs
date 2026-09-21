use crate::models::ZenithError;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use zenith_platform::path_algebra::{self, PathFlavor};
use zenith_platform::PlatformPathsProvider;

pub struct Blacklist;

/// The Windows environment facts the blacklist classifier depends on.
///
/// Reading `std::env` and the Win32 known-folder API inside the classifier made
/// Windows decisions unprovable on any other host. The facts are passed in
/// instead, so [`BlacklistEnvironment::native`] describes the running process
/// and a test can describe a machine with a non-`C:` system drive, a redirected
/// Documents folder, or a hostile UNC profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlacklistEnvironment {
    pub home: Option<String>,
    pub temp_dir: String,
    pub known_content_dirs: Vec<String>,
    pub system_roots: Vec<String>,
    pub local_app_data: Option<String>,
    pub roaming_app_data: Option<String>,
}

impl BlacklistEnvironment {
    /// Describes a stated environment instead of the running process.
    ///
    /// The classifier must reach the same verdict for the environment the
    /// caller is acting on, not for the machine that happens to run the code:
    /// the manifest lint, the plan verifier, and the scanner all classify paths
    /// that came from an injected environment.
    pub fn from_environment(environment: &zenith_platform::PlatformEnvironment) -> Self {
        let text = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
        let known_content_dirs = zenith_platform::KnownFolder::ALL
            .into_iter()
            .filter_map(|folder| environment.content_dir(folder.token()))
            .map(text)
            .collect();
        Self {
            home: environment.user_home().map(text),
            temp_dir: environment.temp_dir().to_string_lossy().into_owned(),
            known_content_dirs,
            // Only the administrator-writable install roots are protected as
            // trees. The application-data roots are protected as exact
            // locations instead: a redirected `LOCALAPPDATA` on a corporate
            // machine holds the very caches Zenith is meant to clean, and
            // treating the whole tree as a system root refused every one of
            // them.
            system_roots: [environment.program_files(), environment.program_data()]
                .into_iter()
                .flatten()
                .map(text)
                .collect(),
            local_app_data: environment.local_app_data().map(text),
            roaming_app_data: environment.roaming_app_data().map(text),
        }
    }

    /// Describes the running process. Only the composition boundary should
    /// prefer this over a stated environment.
    pub fn native() -> Self {
        let paths = zenith_platform::NativePlatformPaths::new();
        Self {
            home: paths.home().map(|path| path.to_string_lossy().into_owned()),
            temp_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            known_content_dirs: ["downloads", "desktop", "documents", "movies"]
                .into_iter()
                .filter_map(|token| paths.content_dir(token))
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            system_roots: [
                "SystemRoot",
                "windir",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramW6432",
                "ProgramData",
            ]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(|value| PathBuf::from(value).to_string_lossy().into_owned())
            .collect(),
            local_app_data: std::env::var_os("LOCALAPPDATA")
                .map(|value| PathBuf::from(value).to_string_lossy().into_owned()),
            roaming_app_data: std::env::var_os("APPDATA")
                .map(|value| PathBuf::from(value).to_string_lossy().into_owned()),
        }
    }
}

/// The decision the Windows blacklist classifier reached, with the rule that
/// fired so a refusal can explain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlacklistVerdict {
    Allowed,
    Denied(&'static str),
}

impl BlacklistVerdict {
    pub fn is_denied(self) -> bool {
        matches!(self, Self::Denied(_))
    }

    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Allowed => None,
            Self::Denied(reason) => Some(reason),
        }
    }
}

const WINDOWS: PathFlavor = PathFlavor::Windows;

/// Home-relative directories holding credentials, personal communication, or
/// user documents. The Windows profile carries the same names as the POSIX one,
/// so both branches refuse the same relative locations.
const SENSITIVE_RELATIVE: [&str; 23] = [
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".config/gcloud",
    "Library/Keychains",
    "Library/Accounts",
    "Library/Mail",
    "Library/Messages",
    "Library/IdentityServices",
    "Library/Containers/com.apple.mail",
    "Downloads",
    "Desktop",
    "Documents",
    "Pictures",
    "Movies",
    "Music",
    "Videos",
    "Contacts",
    "Searches",
    "Links",
    "Saved Games",
];

/// Classifies a Windows path, using Windows rules on every host.
///
/// Every decision goes through [`path_algebra`], which owns separator
/// equivalence, case folding, verbatim/UNC unwrapping, 8.3 alias ambiguity,
/// alternate data streams, trailing dot/space canonicalization, and reserved
/// device names. Nothing here reads the environment or the current OS, so a
/// macOS or Linux test asserts the Windows verdicts directly.
pub fn classify_windows(path: &str, environment: &BlacklistEnvironment) -> BlacklistVerdict {
    // `\\.\` devices, `\??\` NT names, and `\\?\` prefixes that do not wrap a
    // drive or UNC path cannot be normalized into a comparable location.
    if has_unsupported_windows_namespace(path) {
        return BlacklistVerdict::Denied("unsupported Windows namespace");
    }

    if path.starts_with('\\') && !path.starts_with(r"\\") && !path_algebra::is_root(path, WINDOWS) {
        // `\Windows` and `\cache` are rooted on whichever drive is current for
        // the process. Their identity cannot be established from the spelling,
        // so never let them reach a cleanup operation.
        return BlacklistVerdict::Denied("root-relative Windows path");
    }

    let normalized = path_algebra::normalize(path, WINDOWS);
    if normalized.is_empty() {
        return BlacklistVerdict::Allowed;
    }
    if normalized
        .split(WINDOWS.separator())
        .any(|component| component == "..")
    {
        // Windows clamps `..` above a drive root to the root itself, so an
        // over-traversing path can denote a protected directory while looking
        // harmless. It is refused instead of being resolved.
        return BlacklistVerdict::Denied("path traversal above the filesystem root");
    }

    // Classify the resolved spelling as well: `Z:\a\..` is the drive root, and
    // `..` must not be able to smuggle a protected tail past the rules.
    if let Some(root) = path_algebra::protected_root(&normalized, WINDOWS) {
        return BlacklistVerdict::Denied(root.reason());
    }

    if path_algebra::contains_short_name(path, WINDOWS) {
        return BlacklistVerdict::Denied("unresolvable 8.3 short name");
    }

    // On the resolved spelling, so a `..` component is not mistaken for a
    // trailing dot.
    if path_algebra::has_trailing_dot_or_space(&normalized, WINDOWS) {
        return BlacklistVerdict::Denied("component with a trailing dot or space");
    }
    if path_algebra::has_alternate_data_stream(path, WINDOWS) {
        return BlacklistVerdict::Denied("alternate data stream");
    }

    // Component rules run on the unresolved spelling, so a `.git` directory
    // cannot be hidden behind a `..`.
    let canonical =
        path_algebra::canonical_separators(&path_algebra::strip_verbatim(path, WINDOWS), WINDOWS);
    for component in canonical.split('\\') {
        if component.eq_ignore_ascii_case(".git") {
            return BlacklistVerdict::Denied("Git metadata directory");
        }
        if path_algebra::is_reserved_device_name(component) {
            return BlacklistVerdict::Denied("reserved Windows device name");
        }
    }

    let home = environment.home.as_deref().filter(|home| !home.is_empty());
    if home.is_none() {
        return BlacklistVerdict::Denied("user home is unavailable");
    }
    if let Some(home) = home {
        if path_algebra::equal(path, home, WINDOWS) {
            return BlacklistVerdict::Denied("user home");
        }
        // The whole AppData directory and its Local/Roaming children.
        let app_data = join_windows(home, "AppData");
        for candidate in [
            app_data.clone(),
            join_windows(&app_data, "Local"),
            join_windows(&app_data, "Roaming"),
        ] {
            if path_algebra::equal(path, &candidate, WINDOWS) {
                return BlacklistVerdict::Denied("application data directory");
            }
        }
        for candidate in [
            environment.local_app_data.as_deref(),
            environment.roaming_app_data.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if path_algebra::equal(path, candidate, WINDOWS) {
                return BlacklistVerdict::Denied("application data directory");
            }
        }
    }

    if !environment.temp_dir.is_empty() && path_algebra::equal(path, &environment.temp_dir, WINDOWS)
    {
        return BlacklistVerdict::Denied("temporary directory root");
    }

    if let Some(home) = home {
        if path_algebra::contains(home, path, WINDOWS) {
            for relative in SENSITIVE_RELATIVE {
                if path_algebra::contains(&join_windows(home, relative), path, WINDOWS) {
                    return BlacklistVerdict::Denied("sensitive user directory");
                }
            }
            // Resolved content folders and system roots below also apply inside the profile.
        }
    }

    // Known Folder Move and administrator redirection put a content folder
    // inside or outside the literal profile; the resolved location wins over its spelling.
    for directory in &environment.known_content_dirs {
        if path_algebra::contains(directory, path, WINDOWS) {
            return BlacklistVerdict::Denied("resolved content folder");
        }
    }

    // SystemRoot, windir, ProgramFiles, ProgramFiles(x86), ProgramW6432, and
    // ProgramData as the platform actually reported them.
    for root in &environment.system_roots {
        if path_algebra::contains(root, path, WINDOWS) {
            return BlacklistVerdict::Denied("system root");
        }
    }

    // POSIX prefixes (`/usr`, `/etc`, `/System`, …) are deliberately absent:
    // they are POSIX-only rules and `is_blacklisted_posix` applies them.
    BlacklistVerdict::Allowed
}

fn join_windows(base: &str, child: &str) -> String {
    format!(
        r"{}\{}",
        base.trim_end_matches(['\\', '/']),
        child.replace('/', r"\")
    )
}

/// True when the path uses a namespace this application refuses to act on:
/// `\\.\` devices, `\??\` NT names, or a `\\?\` verbatim prefix that wraps
/// neither a drive nor a UNC path (and so cannot be normalized).
fn has_unsupported_windows_namespace(path: &str) -> bool {
    if path_algebra::is_unsupported_namespace(path, WINDOWS) {
        return true;
    }
    let normalized = path.replace('/', r"\");
    let Some(remainder) = normalized.strip_prefix(r"\\?\") else {
        return false;
    };
    let chars: Vec<char> = remainder.chars().collect();
    let is_drive =
        chars.len() >= 3 && chars[0].is_ascii_alphabetic() && chars[1] == ':' && chars[2] == '\\';
    let upper = remainder.to_ascii_uppercase();
    let is_unc = upper.strip_prefix(r"UNC\").is_some_and(|tail| {
        let mut components = tail.split('\\').filter(|part| !part.is_empty());
        components.next().is_some() && components.next().is_some()
    });
    !is_drive && !is_unc
}

impl Blacklist {
    /// System and user directory paths that must NEVER be deleted under any
    /// circumstances, decided for the environment the caller is acting on.
    pub fn is_blacklisted_with(
        path: &Path,
        environment: &zenith_platform::PlatformEnvironment,
    ) -> bool {
        let described = BlacklistEnvironment::from_environment(environment);
        // The described flavor chooses the rule set, not the host that happens
        // to run the check: a simulated Windows environment must be classified
        // by Windows rules on every runner.
        if environment.flavor().is_windows() {
            Self::is_blacklisted_windows(path, &described)
        } else {
            Self::is_blacklisted_posix(
                path,
                &described,
                environment.platform() == zenith_core::domain::platform::PlatformKind::Macos,
            )
        }
    }

    /// Windows classification, delegated to the environment-independent
    /// classifier so the rules execute on every host.
    fn is_blacklisted_windows(path: &Path, described: &BlacklistEnvironment) -> bool {
        // The classifier receives the raw spelling: it accepts only a verbatim
        // prefix that wraps a drive or UNC path and refuses the rest, and
        // normalizing first would strip `\\?\GLOBALROOT\...` into a harmless
        // looking relative path before that rule could see it.
        Self::windows_with_alias_resolution(path, described, |path| {
            std::fs::canonicalize(path).ok()
        })
    }

    fn windows_with_alias_resolution(
        path: &Path,
        described: &BlacklistEnvironment,
        resolve: impl FnOnce(&Path) -> Option<PathBuf>,
    ) -> bool {
        let text = path.to_string_lossy();
        // Windows itself can supply an 8.3 profile in TEMP. Resolve an existing
        // alias before classification; unresolved or still-ambiguous names stay
        // refused. The independent symlink and identity guards still apply.
        let verdict = classify_windows(&text, described);
        if matches!(
            verdict.reason(),
            Some("unresolvable 8.3 short name" | "unresolvable 8.3 short name under a drive root")
        ) {
            let Some(resolved) = resolve(path) else {
                return true;
            };
            return classify_windows(&resolved.to_string_lossy(), described).is_denied();
        }
        verdict.is_denied()
    }

    /// macOS protection uses a conservative comparison key even on case-sensitive
    /// volumes: refusing an alias is preferable to missing protected user state.
    /// Linux keeps byte-exact names. This key never grants deletion authority.
    fn is_blacklisted_posix(path: &Path, described: &BlacklistEnvironment, macos: bool) -> bool {
        let key = |value: &str| -> PathBuf {
            let normalized = path_algebra::normalize(value, PathFlavor::Posix);
            if macos {
                PathBuf::from(
                    normalized
                        .nfd()
                        .flat_map(char::to_lowercase)
                        .collect::<String>(),
                )
            } else {
                PathBuf::from(normalized)
            }
        };
        let Some(raw_path) = path.to_str() else {
            return true;
        };
        let Some(home) = described
            .home
            .as_deref()
            .filter(|home| home.starts_with('/'))
        else {
            return true;
        };
        if !path_algebra::is_absolute(raw_path, PathFlavor::Posix) {
            return true;
        }
        let path = key(raw_path);
        let home = key(home);
        if path == Path::new("/") || path == home || path == key(&described.temp_dir) {
            return true;
        }
        if path.components().any(|part| part.as_os_str() == ".git") {
            return true;
        }
        // Known folders may be redirected inside OR outside the home directory.
        for root in described
            .known_content_dirs
            .iter()
            .chain(&described.system_roots)
        {
            if path.starts_with(key(root)) {
                return true;
            }
        }
        for relative in SENSITIVE_RELATIVE {
            if path.starts_with(home.join(key(relative))) {
                return true;
            }
        }
        for root in [&described.local_app_data, &described.roaming_app_data]
            .into_iter()
            .flatten()
        {
            if path == key(root) {
                return true;
            }
        }
        if ["AppData", "AppData/Local", "AppData/Roaming"]
            .iter()
            .any(|relative| path == home.join(key(relative)))
        {
            return true;
        }
        // Mount containers and each mounted volume itself are never cleanup units.
        for root in ["/Volumes", "/mnt", "/media"] {
            let root = key(root);
            if path == root || path.parent() == Some(root.as_path()) {
                return true;
            }
        }
        // The current home can reside under /var or on a mounted volume.
        if path.starts_with(&home) {
            return false;
        }
        for root in ["/Users", "/home"] {
            if path.starts_with(key(root)) {
                return true;
            }
        }
        // Component comparisons prevent /tmp-neighbor from inheriting /tmp's exception.
        for root in [
            "/tmp",
            "/private/tmp",
            "/var/folders",
            "/private/var/folders",
        ] {
            let root = key(root);
            if path == root {
                return true;
            }
            if path.starts_with(root) {
                return false;
            }
        }
        [
            "/System",
            "/bin",
            "/sbin",
            "/usr",
            "/etc",
            "/var",
            "/private",
            "/Applications",
            "/Library",
            "/Network",
            "/dev",
            "/cores",
            "/opt",
            "/proc",
            "/sys",
            "/boot",
            "/run",
        ]
        .iter()
        .any(|root| path.starts_with(key(root)))
    }

    /// Verifies that a target path is completely safe from the blacklist for
    /// the environment the caller is acting on.
    pub fn validate_with(
        path: &Path,
        environment: &zenith_platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        // Resolve parent components to catch ../ attacks
        let normalized = zenith_platform::path_algebra::normalize_lexical(path);

        if Self::is_blacklisted_with(&normalized, environment) {
            return Err(ZenithError::BlacklistedPath(
                path.to_string_lossy().to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_aliases_require_resolution_and_recheck_the_resolved_location() {
        let environment = windows_environment();
        let alias = Path::new(r"D:\Users\ME~1\AppData\Local\Temp\cache");
        assert!(!Blacklist::windows_with_alias_resolution(
            alias,
            &environment,
            |_| { Some(PathBuf::from(r"D:\Users\me\AppData\Local\Temp\cache")) }
        ));
        assert!(Blacklist::windows_with_alias_resolution(
            alias,
            &environment,
            |_| None
        ));
        assert!(Blacklist::windows_with_alias_resolution(
            alias,
            &environment,
            |_| { Some(PathBuf::from(r"D:\Users\me\Documents\private")) }
        ));
        assert!(Blacklist::windows_with_alias_resolution(
            alias,
            &environment,
            |_| Some(alias.into())
        ));
    }

    fn posix_environment() -> BlacklistEnvironment {
        BlacklistEnvironment {
            home: Some("/Users/José".into()),
            temp_dir: "/private/tmp/zenith-user".into(),
            known_content_dirs: vec!["/Users/José/OneDrive/Personal".into()],
            system_roots: vec![],
            local_app_data: None,
            roaming_app_data: None,
        }
    }

    #[test]
    fn macos_protection_handles_case_and_canonical_unicode_equivalence() {
        let environment = posix_environment();
        for path in [
            "/users/JOSE\u{301}/documents/private.txt",
            "/Users/José/ONEDRIVE/personal/letter.txt",
            "/Users/José/downloads/archive.zip",
            "/Users/José/.SSH/id_ed25519",
            "/Users/José/work/.GIT/config",
        ] {
            assert!(
                Blacklist::is_blacklisted_posix(Path::new(path), &environment, true),
                "{path}"
            );
        }
        assert!(!Blacklist::is_blacklisted_posix(
            Path::new("/Users/José/Library/Caches/tool/data"),
            &environment,
            true
        ));
    }

    #[test]
    fn unix_names_do_not_inherit_windows_alias_rules() {
        let environment = posix_environment();
        for path in [
            "/Users/José/cache/2026-09-21T12:00:00",
            "/Users/José/cache/entry.",
            "/Users/José/cache/entry ",
        ] {
            assert!(
                !Blacklist::is_blacklisted_posix(Path::new(path), &environment, false),
                "{path}"
            );
        }
        assert!(!Blacklist::is_blacklisted_posix(
            Path::new("/Users/José/documents/cache"),
            &environment,
            false
        ));
    }

    #[test]
    fn missing_home_and_mount_roots_fail_closed() {
        let mut environment = posix_environment();
        for path in [
            "/Volumes",
            "/Volumes/Data",
            "/mnt",
            "/mnt/disk",
            "/media/drive",
            "/home/other/data",
            "/Users/other/data",
        ] {
            assert!(
                Blacklist::is_blacklisted_posix(Path::new(path), &environment, true),
                "{path}"
            );
        }
        environment.home = None;
        assert!(Blacklist::is_blacklisted_posix(
            Path::new("/tmp/cache"),
            &environment,
            true
        ));
        let mut windows = windows_environment();
        windows.home = None;
        assert_eq!(
            classify_windows(r"D:\cache\item", &windows),
            BlacklistVerdict::Denied("user home is unavailable")
        );
    }

    #[test]
    fn windows_resolved_content_inside_home_remains_protected() {
        let mut environment = windows_environment();
        environment.known_content_dirs = vec![r"D:\Users\me\OneDrive\Personal".into()];
        assert_eq!(
            classify_windows(r"d:\users\ME\onedrive\PERSONAL\letter.txt", &environment),
            BlacklistVerdict::Denied("resolved content folder")
        );
        assert_eq!(
            classify_windows(r"D:\Users\me\AppData\Local\tool\cache", &environment),
            BlacklistVerdict::Allowed
        );
    }

    /// A machine whose system drive is not `C:` and whose Documents folder was
    /// redirected outside the profile. Every expectation below is stated in
    /// terms of these facts, never in terms of the host that runs the test.
    fn windows_environment() -> BlacklistEnvironment {
        BlacklistEnvironment {
            home: Some(r"D:\Users\me".to_string()),
            temp_dir: r"D:\Users\me\AppData\Local\Temp".to_string(),
            known_content_dirs: vec![r"D:\Redirected\OneDrive\Documents".to_string()],
            system_roots: vec![r"Z:\Windows".to_string(), r"D:\Tools\System".to_string()],
            local_app_data: Some(r"D:\Users\me\AppData\Local".to_string()),
            roaming_app_data: Some(r"D:\Users\me\AppData\Roaming".to_string()),
        }
    }

    /// A Windows machine whose application data was redirected off the system
    /// drive, which is ordinary in managed corporate profiles.
    fn redirected_app_data_environment() -> BlacklistEnvironment {
        BlacklistEnvironment {
            home: Some(r"D:\Users\me".to_string()),
            temp_dir: r"E:\Profiles\me\AppData\Local\Temp".to_string(),
            known_content_dirs: Vec::new(),
            system_roots: vec![
                r"C:\Program Files".to_string(),
                r"C:\ProgramData".to_string(),
            ],
            local_app_data: Some(r"E:\Profiles\me\AppData\Local".to_string()),
            roaming_app_data: Some(r"E:\Profiles\me\AppData\Roaming".to_string()),
        }
    }

    #[test]
    fn the_public_wrapper_refuses_every_unsupported_namespace() {
        // `\\?\` is only meaningful when it wraps a drive or UNC path. A
        // wrapper that normalized before classifying would strip
        // `\\?\GLOBALROOT\...` into a harmless looking relative path and
        // allow it, so the check has to see the raw spelling.
        let environment = windows_environment();
        for path in [
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1",
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1\Windows",
            r"\\.\PhysicalDrive0",
            r"//./PhysicalDrive0",
            r"\??\C:\Windows",
            r"\\?\UNC\server",
        ] {
            let verdict = classify_windows(path, &environment);
            assert!(
                verdict.is_denied(),
                "expected {path} to be denied, got {verdict:?}"
            );
        }

        // A verbatim prefix that does wrap a drive or UNC path stays usable.
        assert_eq!(
            classify_windows(r"\\?\C:\Users\me\dev\cache", &environment),
            BlacklistVerdict::Allowed
        );
        assert!(classify_windows(r"\\?\C:\Windows\System32", &environment).is_denied());
    }

    #[test]
    fn a_redirected_application_data_root_does_not_protect_its_whole_tree() {
        // The root itself is protected as a location.
        let environment = redirected_app_data_environment();
        for path in [
            r"E:\Profiles\me\AppData\Local",
            r"E:\Profiles\me\AppData\Roaming",
            r"E:\Profiles\me\AppData\Local\",
        ] {
            assert!(
                classify_windows(path, &environment).is_denied(),
                "expected the application-data root {path} to be denied"
            );
        }

        // Its caches are exactly what the cleanup features operate on, so they
        // must stay cleanable even though the root moved off the system drive.
        for path in [
            r"E:\Profiles\me\AppData\Local\D3DSCache",
            r"E:\Profiles\me\AppData\Local\NVIDIA\DXCache",
            r"E:\Profiles\me\AppData\Roaming\Vendor\cache\item.bin",
        ] {
            assert_eq!(
                classify_windows(path, &environment),
                BlacklistVerdict::Allowed,
                "expected {path} to stay cleanable"
            );
        }
    }

    fn denied(path: &str) -> &'static str {
        let verdict = classify_windows(path, &windows_environment());
        verdict
            .reason()
            .unwrap_or_else(|| panic!("expected {path:?} to be denied, got {verdict:?}"))
    }

    fn allowed(path: &str) {
        let verdict = classify_windows(path, &windows_environment());
        assert_eq!(
            verdict,
            BlacklistVerdict::Allowed,
            "expected {path:?} to stay cleanable, got {verdict:?}"
        );
    }

    #[test]
    fn every_protected_root_is_denied_on_any_drive_letter() {
        for drive in ["C:", "D:", "Z:"] {
            for spelling in [drive.to_string(), format!(r"{drive}\")] {
                assert_eq!(denied(&spelling), "filesystem root", "spelling {spelling}");
            }
            for (tail, reason) in [
                (r"\Windows", "Windows directory"),
                (r"\Windows\System32", "Windows directory"),
                (r"\Program Files", "Program Files"),
                (r"\Program Files (x86)", "Program Files (x86)"),
                (r"\ProgramData", "ProgramData"),
                (r"\ProgramData\Vendor\app", "ProgramData"),
            ] {
                let path = format!("{drive}{tail}");
                assert_eq!(denied(&path), reason, "path {path}");
            }
            // Only the `X:\Users` root itself; its descendants stay cleanable.
            assert_eq!(denied(&format!(r"{drive}\Users")), "users root");
        }
        assert_eq!(denied("/"), "filesystem root");
        assert_eq!(denied(r"\"), "filesystem root");
    }

    #[test]
    fn short_name_alias_under_a_drive_root_is_denied_without_the_volume() {
        for drive in ["C:", "D:", "Z:"] {
            for alias in [r"\PROGRA~1", r"\PROGRA~1\Vendor\app.exe", r"\DOCUME~1"] {
                let path = format!("{drive}{alias}");
                assert_eq!(
                    denied(&path),
                    "unresolvable 8.3 short name under a drive root",
                    "path {path}"
                );
            }
        }
        assert_eq!(
            denied(r"D:\Users\me\DOCUME~1\file.txt"),
            "unresolvable 8.3 short name"
        );
        // A long name that merely contains a tilde is not an alias.
        allowed(r"D:\Users\me\projects\notes~2024\file.txt");
    }

    #[test]
    fn verbatim_and_unc_spellings_reach_the_same_verdict() {
        assert_eq!(denied(r"D:\Windows"), denied(r"\\?\D:\Windows"));
        assert_eq!(denied(r"Z:\Windows"), denied(r"\\?\Z:\Windows"));
        assert_eq!(
            denied(r"\\server\share\Windows"),
            denied(r"\\?\UNC\server\share\Windows")
        );
        allowed(r"\\?\D:\Users\me\projects\repo");
        allowed(r"\\?\UNC\server\share\projects\repo");
        allowed(r"//?/uNc/server/share/projects/repo");
        allowed(r"\\server\share\projects\repo");

        // Device and NT namespaces, and verbatim prefixes that wrap neither a
        // drive nor a UNC path, are never acted on.
        assert_eq!(
            denied(r"\\.\PhysicalDrive0"),
            "unsupported Windows namespace"
        );
        assert_eq!(
            denied(r"\??\D:\Users\me\projects"),
            "unsupported Windows namespace"
        );
        assert_eq!(
            denied(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1"),
            "unsupported Windows namespace"
        );
        assert_eq!(denied(r"//?/GLOBALROOT/x"), "unsupported Windows namespace");
        assert_eq!(
            denied(r"//./PhysicalDrive0"),
            "unsupported Windows namespace"
        );
        assert_eq!(denied(r"\Windows\System32"), "root-relative Windows path");
        assert_eq!(denied(r"\Users\me\cache"), "root-relative Windows path");
        for root in [r"\\server", r"\\server\share", r"\\?\UNC\server\share"] {
            assert_eq!(denied(root), "filesystem root", "UNC root {root}");
        }
    }

    #[test]
    fn streams_trailing_aliases_and_device_names_are_denied_inside_the_profile() {
        assert_eq!(
            denied(r"D:\Users\me\projects\file.txt:stream"),
            "alternate data stream"
        );
        assert_eq!(
            denied(r"D:\Users\me\projects\cache."),
            "component with a trailing dot or space"
        );
        assert_eq!(
            denied(r"D:\Users\me\projects\cache "),
            "component with a trailing dot or space"
        );
        assert_eq!(
            denied(r"D:\Users\me\projects\nul.txt"),
            "reserved Windows device name"
        );
        assert_eq!(
            denied(r"Z:\projects\LPT1.log"),
            "reserved Windows device name"
        );
        allowed(r"D:\Users\me\projects\file.txt");
        allowed(r"D:\Users\me\projects\console.log");
    }

    #[test]
    fn git_metadata_is_denied_whatever_its_case() {
        for path in [
            r"D:\Users\me\projects\app\.git",
            r"D:\Users\me\projects\app\.git\config",
            r"Z:\projects\Repo\.GIT\objects",
            r"Z:\projects\Repo\.Git",
        ] {
            assert_eq!(denied(path), "Git metadata directory", "path {path}");
        }
        allowed(r"D:\Users\me\projects\git-notes\file.txt");
    }

    #[test]
    fn traversal_that_normalizes_into_a_system_directory_is_denied() {
        assert_eq!(denied(r"D:\Users\me\..\..\Windows"), "Windows directory");
        assert_eq!(
            denied(r"D:\Users\me\Documents\..\..\..\Windows\System32"),
            "Windows directory"
        );
        assert_eq!(
            denied(r"D:\Users\me\projects\..\..\..\Program Files\Vendor"),
            "Program Files"
        );
        assert_eq!(denied(r"Z:\Users\me\..\..\ProgramData\app"), "ProgramData");
        // The shape a signature placeholder produces after expansion:
        // `${USER_HOME}/../../Windows` resolved against the stated profile.
        let home = windows_environment().home.expect("stated home");
        assert_eq!(
            denied(&format!("{home}/../../Windows/System32")),
            "Windows directory"
        );
        // More `..` than the path has components: Windows would clamp it to the
        // drive root, so the unresolved traversal is refused outright.
        assert_eq!(
            denied(r"D:\Users\me\projects\..\..\..\..\Program Files\Vendor"),
            "path traversal above the filesystem root"
        );
        // Traversal that stays inside the profile is still the same location.
        allowed(r"D:\Users\me\projects\..\projects\repo");
    }

    /// The shell's own Recycle Bin store is refused on every volume: the only
    /// supported way to reclaim its space is the interface that maintains its
    /// index, which is never a filesystem delete.
    #[test]
    fn the_recycle_bin_store_is_denied_whatever_its_spelling() {
        for path in [
            r"C:\$Recycle.Bin",
            r"D:\$Recycle.Bin\S-1-5-21-1",
            r"Z:\$RECYCLE.BIN\S-1-5-21-1\$R1234.txt",
            r"D:\RECYCLER",
        ] {
            assert_eq!(denied(path), "Recycle Bin store", "path {path}");
        }
        // The same name below the drive root is an ordinary directory.
        allowed(r"D:\Users\me\projects\$Recycle.Bin");
    }

    #[test]
    fn home_app_data_and_sensitive_directories_are_denied() {
        assert_eq!(denied(r"D:\Users\me"), "user home");
        for path in [
            r"D:\Users\me\AppData",
            r"D:\Users\me\AppData\Local",
            r"D:\Users\me\AppData\Roaming",
        ] {
            assert_eq!(denied(path), "application data directory", "path {path}");
        }
        assert_eq!(
            denied(r"D:\Users\me\AppData\Local\Temp"),
            "temporary directory root"
        );
        for path in [
            r"D:\Users\me\.ssh",
            r"D:\Users\me\.ssh\id_rsa",
            r"D:\Users\me\.kube\config",
            r"D:\Users\me\.config\gcloud\credentials.db",
            r"D:\Users\me\Library\Keychains\login.keychain",
            r"D:\Users\me\Desktop",
            r"D:\Users\me\Documents",
            r"D:\Users\me\Saved Games\slot.sav",
            r"d:\users\ME\documents",
        ] {
            assert_eq!(denied(path), "sensitive user directory", "path {path}");
        }
    }

    #[test]
    fn a_deep_user_path_stays_allowed() {
        for path in [
            r"D:\Users\me\projects\repo\src\main.rs",
            r"D:\Users\me\AppData\Local\Temp\zenith-1234\cache",
            r"D:\Users\me\.cargo\registry\cache",
            r"D:\Users\me\Documents-backup\notes.txt",
            r"D:\Users\me\Desktop-backup",
        ] {
            allowed(path);
        }
    }

    #[test]
    fn redirected_content_folder_outside_the_profile_is_denied() {
        assert_eq!(
            denied(r"D:\Redirected\OneDrive\Documents"),
            "resolved content folder"
        );
        assert_eq!(
            denied(r"D:\Redirected\OneDrive\Documents\report.docx"),
            "resolved content folder"
        );
        // A sibling with a longer name is not inside the resolved folder.
        allowed(r"D:\Redirected\OneDrive\Documents-backup\report.docx");
    }

    #[test]
    fn environment_system_roots_are_denied_even_without_a_known_tail() {
        assert_eq!(denied(r"D:\Tools\System"), "system root");
        assert_eq!(denied(r"D:\Tools\System\bin\tool.exe"), "system root");
        allowed(r"D:\Tools\System-notes\file.txt");
    }

    #[test]
    fn posix_system_prefixes_are_not_applied_to_windows_paths() {
        for path in ["/usr/bin", "/etc/passwd", "/bin/sh", "/System/Library"] {
            allowed(path);
        }
    }

    #[test]
    fn an_empty_path_is_not_a_protected_location() {
        allowed("");
    }
}
