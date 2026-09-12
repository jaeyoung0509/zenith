use crate::models::ZenithError;
use crate::platform::path_algebra::{self, PathFlavor};
use crate::platform::PlatformPathsProvider;
use std::path::{Path, PathBuf};

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
    pub fn from_environment(environment: &crate::platform::PlatformEnvironment) -> Self {
        let text = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
        let known_content_dirs = crate::platform::KnownFolder::ALL
            .into_iter()
            .filter_map(|folder| environment.content_dir(folder.token()))
            .map(text)
            .collect();
        Self {
            home: environment.user_home().map(text),
            temp_dir: environment.temp_dir().to_string_lossy().into_owned(),
            known_content_dirs,
            system_roots: [
                environment.program_files(),
                environment.program_data(),
                environment.local_app_data(),
                environment.roaming_app_data(),
            ]
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
        let paths = crate::platform::NativePlatformPaths::new();
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
const SENSITIVE_RELATIVE: [&str; 22] = [
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
            // A path inside the profile that is not one of the protected
            // subdirectories stays cleanable: the redirected-folder and
            // system-root rules below must not re-open the profile.
            return BlacklistVerdict::Allowed;
        }
    }

    // Known Folder Move and administrator redirection put a content folder
    // outside the literal profile; the resolved location wins over its spelling.
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
    let is_unc = remainder.to_ascii_uppercase().starts_with(r"UNC\");
    !is_drive && !is_unc
}

impl Blacklist {
    /// System and user directory paths that must NEVER be deleted under any
    /// circumstances, decided for the environment the caller is acting on.
    pub fn is_blacklisted_with(
        path: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> bool {
        let described = BlacklistEnvironment::from_environment(environment);
        // The described flavor chooses the rule set, not the host that happens
        // to run the check: a simulated Windows environment must be classified
        // by Windows rules on every runner.
        if environment.flavor().is_windows() {
            Self::is_blacklisted_windows(path, &described)
        } else {
            Self::is_blacklisted_posix(path, &described)
        }
    }

    /// Windows classification, delegated to the environment-independent
    /// classifier so the rules execute on every host.
    fn is_blacklisted_windows(path: &Path, described: &BlacklistEnvironment) -> bool {
        // The raw spelling carries the namespace prefix that normalization
        // rewrites, so classify the raw text first. Normalization then goes
        // through the algebra rather than the host's `Path`, which would treat
        // `C:\\Windows` as a single component on a POSIX runner.
        let raw = path.to_string_lossy();
        if path_algebra::is_unsupported_namespace(&raw, WINDOWS) {
            return true;
        }
        let normalized = path_algebra::normalize(&raw, WINDOWS);
        classify_windows(&normalized, described).is_denied()
    }

    /// POSIX classification keeps byte-exact `Path` semantics.
    fn is_blacklisted_posix(path: &Path, described: &BlacklistEnvironment) -> bool {
        let home = described.home.as_deref().map(PathBuf::from);

        // 1. Exact forbidden root & home
        if path == Path::new("/") {
            return true;
        }

        // Drive roots like C:\ or D:\
        if let Some(path_str) = path.to_str() {
            let trimmed = path_str.trim_end_matches(['\\', '/']);
            if trimmed.len() == 2 && trimmed.ends_with(':') {
                return true;
            }
        }

        if let Some(h) = &home {
            if path == h.as_path() {
                return true;
            }
            // Whole AppData itself or Local/Roaming themselves
            if path == h.join("AppData").as_path()
                || path == h.join("AppData/Local").as_path()
                || path == h.join("AppData\\Local").as_path()
                || path == h.join("AppData/Roaming").as_path()
                || path == h.join("AppData\\Roaming").as_path()
            {
                return true;
            }
        }

        // Whole temp dir itself
        let temp = PathBuf::from(&described.temp_dir);
        if path == temp.as_path() {
            return true;
        }

        // 2. Universal Git protection, ADS, and trailing alias defense
        let path_str = path.to_string_lossy();
        for part in path_str.split(['/', '\\']) {
            if part == ".git" {
                return true;
            }
            if part.ends_with('.') || part.ends_with(' ') {
                return true;
            }
        }

        // Reject alternate data streams (e.g. file.txt:stream or C:\path\file.txt:stream)
        if let Some(colon_pos) = path_str.rfind(':') {
            if colon_pos != 1 {
                return true;
            }
        }

        // 3. User sensitive directories (credentials, keychains, user content)
        if let Some(h) = &home {
            if path.starts_with(h) {
                let sensitive_relative = [
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

                for rel in &sensitive_relative {
                    let sensitive_path = h.join(rel);
                    if path == sensitive_path.as_path() || path.starts_with(&sensitive_path) {
                        return true;
                    }
                }

                return false;
            }
        }

        // 3b. Dynamically resolved known folders (e.g. OneDrive Known Folder Move, redirected Documents/Desktop)
        for known_dir in &described.known_content_dirs {
            let norm_known = PathBuf::from(known_dir);
            if path == norm_known.as_path() || path.starts_with(&norm_known) {
                return true;
            }
        }

        // 4. Allow safe temp directories (/tmp, /private/tmp, /var/folders, /private/var/folders)
        let path_str = path.to_string_lossy();
        if path_str.starts_with("/var/folders")
            || path_str.starts_with("/private/var/folders")
            || path_str.starts_with("/tmp")
            || path_str.starts_with("/private/tmp")
        {
            // Protect root temp folders themselves from direct deletion
            if path == Path::new("/tmp")
                || path == Path::new("/private/tmp")
                || path == Path::new("/var/folders")
                || path == Path::new("/private/var/folders")
            {
                return true;
            }
            return false;
        }

        // 5. Exact users root directory
        let normalized_path_str = path.to_string_lossy().replace('\\', "/");
        if normalized_path_str.eq_ignore_ascii_case("C:/Users") || path == Path::new("/Users") {
            return true;
        }

        // Drive-agnostic protection for Windows system roots on any drive
        // letter. Only the `X:\Users` root itself is protected; its
        // descendants (temp directories, projects, caches) must remain
        // scannable and cleanable.
        let path_normalized_str = normalized_path_str.trim_end_matches('/');
        if path_normalized_str.len() >= 2 && path_normalized_str.as_bytes()[1] == b':' {
            let tail = &path_normalized_str[2..];
            if tail.eq_ignore_ascii_case("/Users") {
                return true;
            }
            for denied in [
                "/Windows",
                "/Program Files",
                "/Program Files (x86)",
                "/ProgramData",
            ] {
                if tail.eq_ignore_ascii_case(denied)
                    || tail
                        .to_ascii_lowercase()
                        .starts_with(&format!("{}/", denied.to_ascii_lowercase()))
                {
                    return true;
                }
            }
        }

        // Environment-derived system roots
        for env_var in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
        ] {
            if let Some(val) = std::env::var_os(env_var).map(PathBuf::from) {
                let norm_sys = crate::platform::NativePlatformPaths::normalize_verbatim_path(&val);
                if path == norm_sys.as_path() || path.starts_with(&norm_sys) {
                    return true;
                }
            }
        }

        // 6. System critical prefixes outside user home and temp
        let system_prefixes = [
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
        ];

        for sys in &system_prefixes {
            let sys_path = Path::new(sys);
            let normalized_system = sys.to_ascii_lowercase();
            let normalized_candidate = normalized_path_str.to_ascii_lowercase();
            if path == sys_path
                || path.starts_with(sys_path)
                || normalized_candidate == normalized_system
                || normalized_candidate.starts_with(&format!("{normalized_system}/"))
            {
                return true;
            }
        }

        false
    }

    /// Verifies that a target path is completely safe from the blacklist for
    /// the environment the caller is acting on.
    pub fn validate_with(
        path: &Path,
        environment: &crate::platform::PlatformEnvironment,
    ) -> Result<(), ZenithError> {
        // Resolve parent components to catch ../ attacks
        let normalized = Self::normalize_path(path);

        if Self::is_blacklisted_with(&normalized, environment) {
            return Err(ZenithError::BlacklistedPath(
                path.to_string_lossy().to_string(),
            ));
        }

        Ok(())
    }

    /// Normalizes path without following symlinks (prevents path traversal `..`)
    pub fn normalize_path(path: &Path) -> PathBuf {
        #[cfg(target_os = "windows")]
        let platform_path = crate::platform::NativePlatformPaths::normalize_verbatim_path(path);
        #[cfg(target_os = "windows")]
        let path = platform_path.as_path();

        let mut components = Vec::new();
        for comp in path.components() {
            match comp {
                std::path::Component::Prefix(p) => components.push(std::path::Component::Prefix(p)),
                std::path::Component::RootDir => components.push(std::path::Component::RootDir),
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if let Some(last) = components.last() {
                        if !matches!(
                            last,
                            std::path::Component::RootDir | std::path::Component::Prefix(_)
                        ) {
                            components.pop();
                        }
                    }
                }
                std::path::Component::Normal(n) => components.push(std::path::Component::Normal(n)),
            }
        }
        components.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
