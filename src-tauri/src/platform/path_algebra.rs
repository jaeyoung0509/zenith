//! Flavor-parameterized path algebra.
//!
//! Windows path semantics — separator equivalence, case folding, verbatim and
//! UNC prefixes, drive-relative roots, reserved device names, and 8.3 alias
//! ambiguity — are expressed here as pure functions over `&str` rather than as
//! `#[cfg(windows)]` branches. Every supported runner therefore exercises the
//! Windows algebra, instead of only the Windows job exercising it and only
//! macOS/Linux asserts that nothing panics.
//!
//! Two rules keep this module trustworthy:
//!
//! * Nothing here reads the environment, the filesystem, or the current OS.
//!   `PathFlavor` is always an explicit parameter, so a caller can present a
//!   simulated Windows environment on any host.
//! * Ambiguous input fails closed. A component that might be an 8.3 alias of a
//!   protected directory is treated as protected rather than assumed benign.

/// Which platform's path syntax and comparison rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathFlavor {
    Posix,
    Windows,
}

impl PathFlavor {
    /// The flavor of the compiled target. Call sites that need the real
    /// platform must use this only at the boundary; the algebra itself never
    /// consults it.
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }

    pub const fn is_windows(self) -> bool {
        matches!(self, Self::Windows)
    }

    /// Canonical separator used when a normalized string is produced.
    pub const fn separator(self) -> char {
        if self.is_windows() {
            '\\'
        } else {
            '/'
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Posix => "posix",
            Self::Windows => "windows",
        }
    }

    fn is_separator(self, ch: char) -> bool {
        if self.is_windows() {
            ch == '\\' || ch == '/'
        } else {
            ch == '/'
        }
    }
}

impl fmt::Display for PathFlavor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

use std::fmt;

/// Why a path is considered protected. Each variant names the rule that fired
/// so a user-facing message can explain the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedRoot {
    /// `/`, `\`, or a drive root such as `C:` / `C:\`.
    FilesystemRoot,
    /// A POSIX system prefix (`/usr`, `/etc`, `/System`, …).
    PosixSystemPrefix,
    /// `<drive>:\Windows` and its descendants.
    WindowsDirectory,
    /// `<drive>:\Program Files` and its descendants.
    ProgramFiles,
    /// `<drive>:\Program Files (x86)` and its descendants.
    ProgramFilesX86,
    /// `<drive>:\ProgramData` and its descendants.
    ProgramData,
    /// `<drive>:\Users` itself (descendants remain cleanable).
    UsersRoot,
    /// A component directly under a drive root that may be an 8.3 alias of a
    /// protected directory. The alias cannot be resolved without the volume, so
    /// the whole subtree is refused rather than assumed safe.
    ShortNameAlias,
}

impl ProtectedRoot {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::FilesystemRoot => "filesystem root",
            Self::PosixSystemPrefix => "system directory",
            Self::WindowsDirectory => "Windows directory",
            Self::ProgramFiles => "Program Files",
            Self::ProgramFilesX86 => "Program Files (x86)",
            Self::ProgramData => "ProgramData",
            Self::UsersRoot => "users root",
            Self::ShortNameAlias => "unresolvable 8.3 short name under a drive root",
        }
    }
}

/// Strips a Windows verbatim prefix: `\\?\C:\x` -> `C:\x`, and
/// `\\?\UNC\server\share` -> `\\server\share`. Device namespaces (`\\.\`,
/// `\??\`) are preserved so callers can classify them as unsupported.
pub fn strip_verbatim(path: &str, flavor: PathFlavor) -> String {
    if !flavor.is_windows() {
        return path.to_string();
    }
    // Windows accepts both separator spellings, including in namespace
    // prefixes. Canonicalize before inspecting the prefix so `//?/UNC/...`
    // cannot take a different path through the safety rules.
    let canonical = path.replace('/', r"\");
    let trimmed = trim_trailing_separators(&canonical, flavor);
    const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
    if trimmed
        .get(..VERBATIM_UNC_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(VERBATIM_UNC_PREFIX))
    {
        let rest = &trimmed[VERBATIM_UNC_PREFIX.len()..];
        return format!(r"\\{rest}");
    }
    if let Some(rest) = trimmed.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    trimmed
}

/// Converts every separator to the flavor's canonical separator.
pub fn canonical_separators(path: &str, flavor: PathFlavor) -> String {
    if !flavor.is_windows() {
        return path.to_string();
    }
    path.replace('/', r"\")
}

/// Joins a tail onto a base using the flavor's own separator, then normalizes
/// the result.
///
/// `Path::join` applies the *host's* rules, so a stated POSIX root joined on a
/// Windows runner came back with backslashes and stopped being recognized as
/// absolute. This keeps the described environment's semantics in charge on
/// every runner.
pub fn join(base: &str, tail: &str, flavor: PathFlavor) -> String {
    let separator = flavor.separator();
    let mut text = canonical_separators(&strip_verbatim(base, flavor), flavor);
    if !text.ends_with(separator) {
        text.push(separator);
    }
    if !tail.is_empty() {
        let tail = canonical_separators(tail, flavor);
        let tail = tail.trim_start_matches(separator);
        text.push_str(tail);
    }
    normalize(&text, flavor)
}

/// Removes trailing separators except when the path is itself a root.
pub fn trim_trailing_separators(path: &str, flavor: PathFlavor) -> String {
    let separator = flavor.separator();
    let trimmed = path.trim_end_matches(|ch| flavor.is_separator(ch));
    if trimmed.is_empty() {
        // `/` and `\` stay as they are; an empty input stays empty.
        return if path.is_empty() {
            String::new()
        } else {
            separator.to_string()
        };
    }
    // `C:` / `\\server` / `\\server\share` must not lose their final
    // separator identity check; `is_drive_root` and `is_unc` work on the
    // trimmed text anyway.
    trimmed.to_string()
}

/// Lexically normalizes a path without touching the filesystem or following
/// links: separators are canonicalized, `.` components are dropped, and `..`
/// components are resolved against the preceding component when one exists.
///
/// The result is idempotent: `normalize(normalize(p)) == normalize(p)`.
pub fn normalize(path: &str, flavor: PathFlavor) -> String {
    let stripped = strip_verbatim(path, flavor);
    let canonical = canonical_separators(&stripped, flavor);
    let separator = flavor.separator();

    let split = split_prefix(&canonical, flavor);

    let mut components: Vec<&str> = Vec::new();
    for component in split.remainder.split(|ch| flavor.is_separator(ch)) {
        if component.is_empty() {
            continue;
        }
        if component == "." {
            continue;
        }
        if component == ".." {
            match components.last() {
                Some(last) if *last != ".." => {
                    components.pop();
                }
                Some(_) => components.push(component),
                None => components.push(component),
            }
            continue;
        }
        components.push(component);
    }

    let mut result = split.prefix;
    let joined = components.join(&separator.to_string());
    if result.is_empty() {
        let is_absolute = canonical.starts_with(separator) && !canonical.is_empty();
        if is_absolute {
            return format!("{separator}{joined}");
        }
        return joined;
    }
    if !split.rooted {
        // `C:cache` is relative to the current directory on `C:` and must not
        // be rewritten into the rooted `C:\cache`. The two name different
        // locations, and rewriting one into the other would let a relative
        // pattern pass an absolute-path check.
        result.push_str(&joined);
        return result;
    }
    if !joined.is_empty() {
        if !result.ends_with(separator) {
            result.push(separator);
        }
        result.push_str(&joined);
    } else if !result.ends_with(separator) {
        result.push(separator);
    }
    result
}

/// A path split into the part that identifies the volume and the rest.
struct PrefixSplit {
    prefix: String,
    remainder: String,
    /// True when the prefix was followed by a separator, which is what makes
    /// `C:\foo` rooted on its drive while `C:foo` stays drive-relative.
    rooted: bool,
}

/// Splits a canonicalized Windows path into its prefix (`\\server\share`,
/// `C:`) and the remainder below it. For POSIX the prefix is empty.
fn split_prefix(canonical: &str, flavor: PathFlavor) -> PrefixSplit {
    if !flavor.is_windows() {
        return PrefixSplit {
            prefix: String::new(),
            remainder: canonical.trim_start_matches('/').to_string(),
            rooted: canonical.starts_with('/'),
        };
    }
    if let Some(rest) = canonical.strip_prefix(r"\\") {
        // UNC: \\server\share\... — the share is part of the prefix.
        let mut parts = rest.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        let remainder = parts.next().unwrap_or_default();
        if server.is_empty() {
            return PrefixSplit {
                prefix: canonical.to_string(),
                remainder: String::new(),
                rooted: true,
            };
        }
        if share.is_empty() {
            return PrefixSplit {
                prefix: format!(r"\\{server}"),
                remainder: String::new(),
                rooted: true,
            };
        }
        return PrefixSplit {
            prefix: format!(r"\\{server}\{share}"),
            remainder: remainder.to_string(),
            rooted: true,
        };
    }
    let mut chars = canonical.chars();
    let first = chars.next();
    let second = chars.next();
    if let (Some(first), Some(':')) = (first, second) {
        if is_drive_letter(first) {
            let after_prefix = &canonical[2..];
            let rooted = after_prefix.starts_with('\\') || after_prefix.starts_with('/');
            let remainder = after_prefix.trim_start_matches(['\\', '/']);
            return PrefixSplit {
                prefix: canonical[..2].to_string(),
                remainder: remainder.to_string(),
                rooted,
            };
        }
    }
    PrefixSplit {
        prefix: String::new(),
        remainder: canonical.trim_start_matches('\\').to_string(),
        rooted: false,
    }
}

fn is_drive_letter(ch: char) -> bool {
    ch.is_ascii_alphabetic()
}

/// Case fold used for comparison. Windows keeps its current fold (Unicode
/// uppercase); POSIX is case sensitive.
pub fn fold(text: &str, flavor: PathFlavor) -> String {
    if flavor.is_windows() {
        text.to_uppercase()
    } else {
        text.to_string()
    }
}

/// Comparison key: normalized, case folded, no trailing separator.
pub fn key(path: &str, flavor: PathFlavor) -> String {
    fold(&normalize(path, flavor), flavor)
}

/// True when both paths denote the same location under the flavor's rules.
pub fn equal(left: &str, right: &str, flavor: PathFlavor) -> bool {
    let left_key = key(left, flavor);
    let right_key = key(right, flavor);
    if flavor.is_windows() {
        return left_key == right_key || drive_root_keys_match(&left_key, &right_key);
    }
    left_key == right_key
}

fn drive_root_keys_match(left: &str, right: &str) -> bool {
    left.trim_end_matches('\\') == right.trim_end_matches('\\')
}

/// Component-boundary containment: `parent` contains `child` when `child`
/// equals `parent` or lives below it. `C:\Program Files (x86)` does not contain
/// `C:\Program Files`.
pub fn contains(parent: &str, child: &str, flavor: PathFlavor) -> bool {
    let parent_key = key(parent, flavor);
    let child_key = key(child, flavor);
    if parent_key == child_key {
        return true;
    }
    let separator = flavor.separator();
    let parent_trimmed = if flavor.is_windows() {
        parent_key.trim_end_matches('\\').to_string()
    } else {
        parent_key.clone()
    };
    if parent_trimmed.is_empty() {
        return false;
    }
    if parent_trimmed == separator.to_string() {
        return child_key.starts_with(separator);
    }
    child_key
        .strip_prefix(&parent_trimmed)
        .is_some_and(|suffix| suffix.starts_with(separator))
}

/// True when the path denotes a filesystem root (`/`, `\`, `C:`, `C:\`) or
/// a UNC share root (`\\server\share`). Incomplete UNC prefixes are treated as
/// broad roots too: callers must fail closed rather than act on an ambiguous
/// server-level spelling.
pub fn is_root(path: &str, flavor: PathFlavor) -> bool {
    let normalized = normalize(path, flavor);
    let trimmed = normalized.trim_end_matches(|ch| flavor.is_separator(ch));
    if trimmed.is_empty() {
        return !path.is_empty();
    }
    if !flavor.is_windows() {
        return false;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() == 2 && is_drive_letter(chars[0]) && chars[1] == ':' {
        return true;
    }
    if chars.len() == 1 && flavor.is_separator(chars[0]) {
        return true;
    }
    if trimmed.starts_with(r"\\") {
        return split_prefix(trimmed, flavor).remainder.is_empty();
    }
    false
}

/// True when the path is absolute under the flavor's own rules.
///
/// This is not `Path::is_absolute`, which always applies the *host's* rules: a
/// simulated `D:\Users\me` is absolute on Windows and is not a relative POSIX
/// path, and the difference decides whether a cleanup target is acceptable.
pub fn is_absolute(path: &str, flavor: PathFlavor) -> bool {
    let stripped = canonical_separators(&strip_verbatim(path, flavor), flavor);
    if !flavor.is_windows() {
        return stripped.starts_with('/');
    }
    if is_unc(&stripped, flavor) {
        return true;
    }
    let chars: Vec<char> = stripped.chars().collect();
    chars.len() >= 3
        && is_drive_letter(chars[0])
        && chars[1] == ':'
        && flavor.is_separator(chars[2])
}

/// True when the path is a complete UNC path (`\\server\share`). A server-only
/// prefix is not a usable absolute path.
pub fn is_unc(path: &str, flavor: PathFlavor) -> bool {
    if !flavor.is_windows() {
        return false;
    }
    let canonical = canonical_separators(path, flavor);
    if !canonical.starts_with(r"\\")
        || canonical.starts_with(r"\\?\")
        || canonical.starts_with(r"\\.\")
    {
        return false;
    }
    let mut components = canonical[2..].split('\\').filter(|part| !part.is_empty());
    components.next().is_some() && components.next().is_some()
}

/// True when the path uses a Windows namespace this application refuses to act
/// on (`\\.\` device and `\??\` NT namespaces).
pub fn is_unsupported_namespace(path: &str, flavor: PathFlavor) -> bool {
    if !flavor.is_windows() {
        return false;
    }
    let canonical = canonical_separators(path, flavor);
    canonical.starts_with(r"\\.\") || canonical.starts_with(r"\??\")
}

/// True for a component that Windows resolves to a device rather than a file,
/// with or without an extension (`CON`, `nul.txt`, `LPT1`).
pub fn is_reserved_device_name(component: &str) -> bool {
    let base = component
        .split(['.', ':'])
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ')
        .trim_end_matches('.');
    if base.is_empty() {
        return false;
    }
    let upper = base.to_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes: Vec<char> = upper.chars().collect();
    if bytes.len() == 4 {
        let prefix = bytes[0];
        let digit = bytes[3];
        if (prefix == 'C' || prefix == 'L')
            && (bytes[1], bytes[2]) == ('O', 'M')
            && digit.is_ascii_digit()
        {
            return true;
        }
        if (prefix, bytes[1]) == ('L', 'P') && bytes[2] == 'T' && digit.is_ascii_digit() {
            return true;
        }
    }
    // Superscript digits (¹²³) are also resolved by the Win32 layer.
    matches!(
        upper.as_str(),
        "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
    )
}

/// True when a component could be an 8.3 short name. The alias target cannot be
/// discovered without the volume, so callers must fail closed.
pub fn looks_like_short_name(component: &str) -> bool {
    if component.is_empty() || component == "." || component == ".." {
        return false;
    }
    // `PROGRA~1`, `DOCUME~1`, `RUNNER~1`
    let (stem, tail) = match component.split_once('~') {
        Some((stem, tail)) => (stem, tail),
        None => return false,
    };
    if stem.is_empty() || stem.len() > 8 {
        return false;
    }
    let digits: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let rest: String = tail.chars().skip(digits.len()).collect();
    let rest = rest
        .split_once('.')
        .map(|(_, extension)| extension.to_string())
        .unwrap_or(rest);
    stem.chars().all(is_short_name_char)
        && rest.chars().all(is_short_name_char)
        && digits.len() <= 6
}

fn is_short_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "_$~!#%&'()-@^`{}".contains(ch)
}

/// First component below the drive root (or the leading component for POSIX),
/// used to decide whether an 8.3 alias could shadow a protected directory.
pub fn first_component(path: &str, flavor: PathFlavor) -> Option<String> {
    let canonical = canonical_separators(&strip_verbatim(path, flavor), flavor);
    let split = split_prefix(&canonical, flavor);
    split
        .remainder
        .split(|ch| flavor.is_separator(ch))
        .find(|component| !component.is_empty())
        .map(str::to_string)
}

/// Classifies a path that must never be deleted. Returns `None` when the path
/// is not protected by a root rule; other rules (sensitive user directories,
/// known folders, temporary-directory policy) are owned by the caller.
pub fn protected_root(path: &str, flavor: PathFlavor) -> Option<ProtectedRoot> {
    if is_root(path, flavor) {
        return Some(ProtectedRoot::FilesystemRoot);
    }
    let canonical = normalize(path, flavor);
    if canonical.is_empty() {
        return None;
    }
    if !flavor.is_windows() {
        for prefix in [
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
        ] {
            if contains(prefix, &canonical, flavor) {
                return Some(ProtectedRoot::PosixSystemPrefix);
            }
        }
        return None;
    }

    // Windows: the protected tails are drive-letter independent, so a machine
    // whose system drive is `D:` is protected exactly like a `C:` machine.
    let split = split_prefix(&canonical, flavor);
    let is_drive_prefixed = split.prefix.len() == 2 && split.prefix.ends_with(':');
    let is_unc_prefixed = split.prefix.starts_with(r"\\");
    if !is_drive_prefixed && !is_unc_prefixed {
        return None;
    }
    if is_drive_prefixed && !split.rooted {
        // `C:Windows` is drive-relative, not the Windows directory. Refuse it
        // instead of classifying a path whose meaning depends on the calling
        // process's current directory.
        return Some(ProtectedRoot::FilesystemRoot);
    }
    let mut components = split.remainder.split('\\').filter(|part| !part.is_empty());
    let first = components.next()?;
    let tail_is_only_component = components.next().is_none();

    let tail_class = if first.eq_ignore_ascii_case("Windows") {
        Some(ProtectedRoot::WindowsDirectory)
    } else if first.eq_ignore_ascii_case("Program Files") {
        Some(ProtectedRoot::ProgramFiles)
    } else if first.eq_ignore_ascii_case("Program Files (x86)") {
        Some(ProtectedRoot::ProgramFilesX86)
    } else if first.eq_ignore_ascii_case("ProgramData") {
        Some(ProtectedRoot::ProgramData)
    } else if first.eq_ignore_ascii_case("Users") && tail_is_only_component {
        Some(ProtectedRoot::UsersRoot)
    } else {
        None
    };
    if tail_class.is_some() {
        return tail_class;
    }
    if looks_like_short_name(first) {
        // `C:\PROGRA~1` may resolve to `C:\Program Files`; refuse the subtree
        // instead of assuming the alias is something harmless.
        return Some(ProtectedRoot::ShortNameAlias);
    }
    None
}

/// True when any component could be an 8.3 alias. Used by callers that cannot
/// resolve the volume but require a canonical identity.
pub fn contains_short_name(path: &str, flavor: PathFlavor) -> bool {
    if !flavor.is_windows() {
        return false;
    }
    normalize(path, flavor)
        .split('\\')
        .any(looks_like_short_name)
}

/// True when a path carries an NTFS alternate data stream (`file.txt:stream`).
/// A single drive colon is not an alternate data stream.
pub fn has_alternate_data_stream(path: &str, flavor: PathFlavor) -> bool {
    if !flavor.is_windows() {
        return false;
    }
    let canonical = canonical_separators(&strip_verbatim(path, flavor), flavor);
    let mut search_from = 0;
    let bytes: Vec<char> = canonical.chars().collect();
    while let Some(offset) = canonical[search_from..].find(':') {
        let index = search_from + canonical[..search_from + offset].chars().count();
        let is_drive_colon = index == 1 && bytes.first().is_some_and(|ch| is_drive_letter(*ch));
        if !is_drive_colon && index != 0 {
            return true;
        }
        search_from += offset + 1;
        if search_from >= canonical.len() {
            break;
        }
    }
    false
}

/// True when a component ends with a dot or a space, which Windows silently
/// strips — the trimmed and untrimmed names then refer to the same file.
pub fn has_trailing_dot_or_space(path: &str, flavor: PathFlavor) -> bool {
    if !flavor.is_windows() {
        return false;
    }
    canonical_separators(&strip_verbatim(path, flavor), flavor)
        .split('\\')
        .any(|component| component.ends_with('.') || component.ends_with(' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: PathFlavor = PathFlavor::Windows;
    const P: PathFlavor = PathFlavor::Posix;

    #[test]
    fn normalization_is_idempotent_for_every_fixture() {
        let fixtures = [
            ("", W),
            ("   ", W),
            (r"C:\Windows\System32\..\..\Users\me\Downloads", W),
            (r"C:/Windows/System32", W),
            (r"\\?\C:\Users\me\AppData\Local\Temp\..\Temp\Zenith", W),
            (r"\\?\UNC\server\share\folder\..\folder", W),
            (r"\\server\share\folder\", W),
            (r"C:\\Windows\\\\System32", W),
            (r"..\..\Windows", W),
            ("/usr/local/../lib", P),
            ("////usr////bin////", P),
            ("/", P),
            ("\u{0}\u{0}\\", W),
            (r"C:\Users\me\ホーム\..\ドキュメント", W),
        ];
        for (path, flavor) in fixtures {
            let once = normalize(path, flavor);
            let twice = normalize(&once, flavor);
            assert_eq!(once, twice, "normalize not idempotent for {path:?}");
        }
    }

    #[test]
    fn case_folding_is_stable_and_idempotent() {
        for sample in ["ΣΊΣΥΦΟΣ", "İstanbul", "straße", "ÄÖÜ", "ドキュメント"] {
            let once = fold(sample, W);
            assert_eq!(once, fold(&once, W), "fold not idempotent for {sample:?}");
        }
        assert_eq!(fold("abc", W), fold("ABC", W));
        assert_eq!(fold("straße", W), fold("STRASSE", W));
        assert_eq!(fold("Ä", W), fold("ä", W));
        // POSIX folding must not erase case.
        assert_ne!(fold("A", P), fold("a", P));
    }

    #[test]
    fn separators_are_equivalent_on_windows_only() {
        assert!(equal(r"C:\Program Files", "C:/Program Files", W));
        assert!(equal(r"C:\Program Files\\", r"C:\Program Files", W));
        assert!(!equal("/usr/bin", "/usr\\bin", P));
    }

    #[test]
    fn containment_respects_component_boundaries() {
        assert!(contains(r"C:\Program Files", r"C:\Program Files\Zenith", W));
        assert!(!contains(
            r"C:\Program Files",
            r"C:\Program Files (x86)\Zenith",
            W
        ));
        assert!(!contains(r"C:\Program", r"C:\Program Files", W));
        assert!(contains("/usr", "/usr/bin", P));
        assert!(!contains("/usr", "/usrlocal", P));
    }

    #[test]
    fn verbatim_and_unc_prefixes_do_not_change_identity() {
        assert_eq!(normalize(r"\\?\C:\Windows", W), normalize(r"C:\Windows", W));
        assert_eq!(
            normalize(r"\\?\UNC\server\share\Windows", W),
            normalize(r"\\server\share\Windows", W)
        );
        assert_eq!(
            normalize(r"//?/uNc/server/share/Windows", W),
            normalize(r"\\server\share\Windows", W)
        );
        assert!(equal(r"\\?\C:\Windows\", r"C:\Windows", W));
        assert!(is_unc(r"\\server\share", W));
        assert!(is_unc(r"//server/share/folder", W));
        assert!(!is_unc(r"\\server", W));
        assert!(!is_unc(r"\\?\C:\Windows", W));
        assert!(is_unsupported_namespace(r"\\.\C:\Windows", W));
        assert!(is_unsupported_namespace(r"//./PhysicalDrive0", W));
        assert!(is_unsupported_namespace(r"\??\C:\Windows", W));
    }

    #[test]
    fn drive_letter_does_not_decide_protection() {
        for drive in ["C:", "D:", "Z:"] {
            let windows = format!(r"{drive}\Windows\System32");
            assert_eq!(
                protected_root(&windows, W),
                Some(ProtectedRoot::WindowsDirectory),
                "{drive} must be protected"
            );
            let program_files = format!(r"{drive}\Program Files (x86)\Zenith");
            assert_eq!(
                protected_root(&program_files, W),
                Some(ProtectedRoot::ProgramFilesX86)
            );
            assert!(protected_root(&format!(r"{drive}\Users\me\Downloads\cache"), W).is_none());
        }
        assert_eq!(
            protected_root(r"D:\Users", W),
            Some(ProtectedRoot::UsersRoot)
        );
        assert_eq!(protected_root(r"C:\Users\me", W), None);
    }

    #[test]
    fn posix_system_prefixes_are_protected_with_descendants() {
        for path in [
            "/usr",
            "/usr/bin",
            "/etc/hosts",
            "/System/Library",
            "/Applications",
            "/Library/Caches",
        ] {
            assert_eq!(
                protected_root(path, P),
                Some(ProtectedRoot::PosixSystemPrefix),
                "{path} must be protected"
            );
        }
        assert_eq!(protected_root("/Users/me/Downloads", P), None);
        assert_eq!(protected_root("relative/path", P), None);
    }

    #[test]
    fn short_name_aliases_fail_closed() {
        assert!(looks_like_short_name("PROGRA~1"));
        assert!(looks_like_short_name("DOCUME~1"));
        assert!(!looks_like_short_name("Program Files"));
        assert!(!looks_like_short_name("~"));
        assert_eq!(
            protected_root(r"C:\PROGRA~1\Zenith", W),
            Some(ProtectedRoot::ShortNameAlias)
        );
        assert_eq!(
            protected_root(r"C:\Users\me\PROGRA~1", W),
            None,
            "a tilde component below the drive root must not block unrelated cleanup"
        );
        assert!(contains_short_name(r"C:\PROGRA~1\Zenith", W));
        assert!(!contains_short_name(r"C:\Program Files\Zenith", W));
        // A short name is never silently equal to the directory it may alias.
        assert_ne!(key(r"C:\PROGRA~1", W), key(r"C:\Program Files", W));
    }

    #[test]
    fn reserved_device_names_and_streams_are_detected() {
        for component in ["CON", "con.txt", "NUL", "COM1", "lpt9.log", "AUX", "COM¹"] {
            assert!(
                is_reserved_device_name(component),
                "{component} is reserved"
            );
        }
        for component in ["console", "CONS", "COM", "LPT", "Program Files"] {
            assert!(
                !is_reserved_device_name(component),
                "{component} is a normal name"
            );
        }
        assert!(has_alternate_data_stream(r"C:\Users\me\file.txt:secret", W));
        assert!(!has_alternate_data_stream(r"C:\Users\me\file.txt", W));
        assert!(has_trailing_dot_or_space(r"C:\Users\me\Downloads ", W));
        assert!(!has_trailing_dot_or_space(r"C:\Users\me\Downloads", W));
    }

    #[test]
    fn drive_relative_paths_stay_relative() {
        // `C:cache` is relative to the current directory on `C:`; `C:\cache`
        // is rooted on the drive. Rewriting one into the other would let a
        // relative pattern pass an absolute-path check.
        assert_eq!(normalize("C:cache", W), "C:cache");
        assert_eq!(normalize(r"C:\cache", W), r"C:\cache");
        assert!(!is_absolute(&normalize("C:cache", W), W));
        assert!(is_absolute(&normalize(r"C:\cache", W), W));
        assert!(!is_absolute(&normalize(r"C:..\Windows", W), W));
        assert_eq!(normalize(r"C:..\Windows", W), r"C:..\Windows");
        assert!(!is_absolute(&normalize(r"D:Users\me", W), W));
        assert!(!is_absolute(&normalize("C:.", W), W));
        // A drive-relative path is never classified as a protected root by
        // accident; it is refused as a root instead.
        assert_eq!(
            protected_root(r"C:Windows", W),
            Some(ProtectedRoot::FilesystemRoot)
        );
        assert_eq!(
            protected_root(r"C:\Windows", W),
            Some(ProtectedRoot::WindowsDirectory)
        );
        // Normalization stays idempotent for relative spellings too.
        for path in ["C:cache", r"C:..\Windows", "C:.", "relative/path"] {
            let once = normalize(path, W);
            assert_eq!(once, normalize(&once, W), "not idempotent for {path:?}");
        }
    }

    #[test]
    fn joining_a_tail_uses_the_described_flavor_not_the_host() {
        // Host `Path::join` would turn these into backslash paths on a Windows
        // runner and stop recognizing them as absolute.
        assert_eq!(
            join("/mnt/data/downloads", "cache", P),
            "/mnt/data/downloads/cache"
        );
        assert_eq!(
            join(r"D:\\Cache\\Documents", r"keep\\file.txt", W),
            r"D:\Cache\Documents\keep\file.txt"
        );
        assert_eq!(
            join(r"\\fileserver\share", "kept", W),
            r"\\fileserver\share\kept"
        );
        // A tail that is already rooted is appended, not substituted.
        assert_eq!(join("/base", "/tail", P), "/base/tail");
        assert_eq!(join("/base", "", P), "/base");
    }

    #[test]
    fn roots_are_recognized_on_both_flavors() {
        for path in ["/", "///", "C:", r"C:\", "D:"] {
            let flavor = if path.starts_with('/') { P } else { W };
            assert!(is_root(path, flavor), "{path} must be a root");
        }
        for path in [r"\\server", r"\\server\share", r"\\?\UNC\server\share\"] {
            assert!(is_root(path, W), "{path} must be a broad UNC root");
            assert_eq!(protected_root(path, W), Some(ProtectedRoot::FilesystemRoot));
        }
        assert!(!is_absolute(r"\\server", W));
        assert!(is_absolute(r"\\server\share", W));
        assert!(!is_root(r"\\server\share\folder", W));
        assert!(!is_root(r"C:\Users", W));
        assert!(!is_root("/Users", P));
    }

    #[test]
    fn pathological_input_does_not_panic() {
        let inputs = [
            "",
            "\\",
            "\\\\",
            "\\\\\\\\?\\\\",
            "::",
            "...",
            "..\\..\\..\\..\\",
            "\u{feff}C:\\Windows",
            &"a".repeat(4096),
            "\u{0}",
            "C:",
            "\\\\server",
            "\\\\server\\",
            "\\\\?\\",
        ];
        for input in inputs {
            for flavor in [W, P] {
                let normalized = normalize(input, flavor);
                let _ = key(input, flavor);
                let _ = contains(input, &normalized, flavor);
                let _ = protected_root(input, flavor);
                let _ = first_component(input, flavor);
                let _ = has_alternate_data_stream(input, flavor);
                let _ = has_trailing_dot_or_space(input, flavor);
                let _ = contains_short_name(input, flavor);
            }
        }
    }

    #[test]
    fn first_component_skips_prefixes_and_empty_segments() {
        assert_eq!(
            first_component(r"C:\\Windows\\System32", W).as_deref(),
            Some("Windows")
        );
        assert_eq!(
            first_component(r"\\server\share\folder", W).as_deref(),
            Some("folder")
        );
        assert_eq!(first_component("///usr//bin", P).as_deref(), Some("usr"));
        assert_eq!(first_component("C:", W), None);
        assert_eq!(first_component("/", P), None);
    }
}
