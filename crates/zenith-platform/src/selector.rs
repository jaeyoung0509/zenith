//! Bounded path selectors for catalog roots.
//!
//! A cleanup catalog has to name roots it cannot spell literally: every browser
//! profile, every packaged application, every Electron app's `GPUCache`. A
//! catalog that lists `Default` finds one profile and misses the rest; a
//! catalog that lists nothing finds nothing.
//!
//! A selector is the smallest notation that closes that gap without becoming a
//! pattern language:
//!
//! - a component that is exactly `*` matches any one directory name;
//! - a component that is exactly `{a,b,c}` matches one of those names;
//! - everything else is literal, and `**`, `Cache*`, and nested braces are
//!   **refused** rather than treated as literals, because a pattern that
//!   silently matches nothing is a catalog entry that silently does nothing;
//! - `..` is refused, because a catalog root never has one.
//!
//! Expansion is bounded, deterministic, and shallow about what it will accept:
//! matches are sorted, capped, and only ever descend into real directories that
//! are not symlinks. The result states whether the cap was reached, so a scan
//! can report a truncated enumeration instead of presenting it as complete.

use crate::path_algebra::{self, PathFlavor, PathParts};
use std::path::{Path, PathBuf};

/// The most matches one pattern may produce.
///
/// A pattern that matches more than this is describing a machine whose layout
/// no catalog entry anticipated; the cap keeps a scan bounded and the outcome
/// reports the truncation rather than silently covering part of it.
pub const SELECTOR_MATCH_LIMIT: usize = 256;

/// A selector that could not be parsed, with the reason a manifest author needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorError {
    pub pattern: String,
    pub reason: String,
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "`{}`: {}", self.pattern, self.reason)
    }
}

/// One component of a selector pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorComponent {
    Literal(String),
    /// `*`: any single directory name.
    AnyOne,
    /// `{a,b}`: one of these names.
    OneOf(Vec<String>),
}

impl SelectorComponent {
    fn matches(&self, name: &str, flavor: PathFlavor) -> bool {
        match self {
            Self::Literal(expected) => path_algebra::equal(expected, name, flavor),
            Self::AnyOne => !name.is_empty(),
            Self::OneOf(names) => names
                .iter()
                .any(|expected| path_algebra::equal(expected, name, flavor)),
        }
    }

    /// The name this component contributes to a concrete path, when it has one.
    pub fn literal_name(&self) -> Option<&str> {
        match self {
            Self::Literal(name) => Some(name.as_str()),
            Self::AnyOne | Self::OneOf(_) => None,
        }
    }
}

/// A pattern that describes one or more concrete roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSelector {
    parts: PathParts,
    components: Vec<SelectorComponent>,
    flavor: PathFlavor,
}

/// An access or I/O failure encountered while expanding a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionFailure {
    pub path: PathBuf,
    pub error: String,
}

/// What an expansion found, and whether it stopped early.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectionOutcome {
    /// Concrete directories the pattern matched, sorted, one entry per match.
    pub matches: Vec<PathBuf>,
    /// True when the match cap stopped the enumeration before it finished.
    pub truncated: bool,
    /// Locations where enumeration failed because of permissions or I/O errors.
    pub failures: Vec<SelectionFailure>,
}

impl SelectionOutcome {
    pub fn record_failure(&mut self, path: PathBuf, error: String) {
        if !self.failures.iter().any(|f| f.path == path) {
            self.failures.push(SelectionFailure { path, error });
        }
    }
}

impl PathSelector {
    /// Whether a pattern contains selector syntax at all.
    ///
    /// A partial wildcard (`Cache*`) also counts: it is selector *syntax* that
    /// [`Self::parse`] refuses, and callers must not treat it as a literal.
    pub fn is_pattern(pattern: &str) -> bool {
        contains_selector_syntax(pattern)
    }

    /// Parses a pattern, or reports why it is not one.
    ///
    /// Returns the parsed selector; a caller that needs to know whether a
    /// pattern contained syntax at all should ask [`Self::is_pattern`] first.
    pub fn parse(pattern: &str, flavor: PathFlavor) -> Result<Self, SelectorError> {
        let refuse = |reason: &str| SelectorError {
            pattern: pattern.to_string(),
            reason: reason.to_string(),
        };

        let parts = path_algebra::split_path(pattern, flavor);
        if parts.components.iter().any(|component| component == "..") {
            return Err(refuse("`..` is not allowed in a catalog root"));
        }
        if parts.components.is_empty() {
            return Err(refuse("the pattern has no components below its root"));
        }

        let mut components = Vec::with_capacity(parts.components.len());
        for component in &parts.components {
            if component.contains('*') || component.contains('{') || component.contains('}') {
                components.push(Self::parse_component(component, &refuse)?);
            } else {
                components.push(SelectorComponent::Literal(component.clone()));
            }
        }

        // A pattern that names no selector component is a literal path: the
        // caller resolves it through the placeholder expander instead.
        if !components.iter().any(|component| {
            matches!(
                component,
                SelectorComponent::AnyOne | SelectorComponent::OneOf(_)
            )
        }) {
            return Err(refuse("the pattern contains no selector component"));
        }

        Ok(Self {
            parts,
            components,
            flavor,
        })
    }

    fn parse_component(
        component: &str,
        refuse: &impl Fn(&str) -> SelectorError,
    ) -> Result<SelectorComponent, SelectorError> {
        if component == "*" {
            return Ok(SelectorComponent::AnyOne);
        }
        if let Some(inner) = component
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        {
            if inner.contains('{') || inner.contains('}') || inner.contains('*') {
                return Err(refuse(
                    "a `{...}` component may only list names, without nesting or wildcards",
                ));
            }
            let names: Vec<String> = inner
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect();
            if names.is_empty() {
                return Err(refuse("a `{...}` component must list at least one name"));
            }
            return Ok(SelectorComponent::OneOf(names));
        }
        Err(refuse(
            "a selector component must be exactly `*` or `{a,b}`; a partial wildcard is not supported",
        ))
    }

    /// The longest literal directory prefix shared by every match.
    ///
    /// `None` when the pattern's root is itself a selector, so a caller that
    /// needs a real directory to start from (the manifest lint) reports the
    /// pattern instead of walking from `/`.
    pub fn static_root(&self) -> Option<String> {
        let mut parts = self.parts.clone();
        let mut components = Vec::new();
        for component in &self.components {
            match component {
                SelectorComponent::Literal(name) => components.push(name.clone()),
                SelectorComponent::AnyOne | SelectorComponent::OneOf(_) => break,
            }
        }
        if components.is_empty() {
            return None;
        }
        parts.components = components;
        Some(path_algebra::join_parts(&parts, self.flavor))
    }

    /// Whether a concrete path matches this pattern.
    ///
    /// Comparison follows the stated flavor, so a Windows pattern matches
    /// `C:\\Users\\me\\AppData\\Local\\Cache` and `c:\\users\\ME\\appdata\\local\\cache`
    /// alike, while a POSIX pattern keeps its case.
    pub fn matches(&self, path: &str, flavor: PathFlavor) -> bool {
        let candidate = path_algebra::split_path(path, flavor);
        if !path_algebra::equal(&candidate.prefix, &self.parts.prefix, flavor) {
            return false;
        }
        if candidate.components.len() != self.components.len() {
            return false;
        }
        self.components
            .iter()
            .zip(candidate.components.iter())
            .all(|(component, name)| component.matches(name, flavor))
    }

    /// The key a concrete match contributes to an item identity.
    ///
    /// The components the pattern itself did not spell out, joined in order, so
    /// two matches of one pattern never share an identity.
    pub fn match_key(&self, path: &str, flavor: PathFlavor) -> Option<String> {
        if !self.matches(path, flavor) {
            return None;
        }
        let candidate = path_algebra::split_path(path, flavor);
        let mut key = Vec::new();
        for (component, name) in self.components.iter().zip(candidate.components.iter()) {
            if component.literal_name().is_none() {
                key.push(name.clone());
            }
        }
        Some(key.join("-"))
    }

    /// Expands the pattern to the directories it matches.
    ///
    /// Each component is resolved against the real filesystem, one level at a
    /// time, and the expansion **never descends through an indirection**: every
    /// component that is not the last one must be a real directory — not a
    /// symlink, and on Windows not a reparse point of any tag — or the branch
    /// stops there.
    ///
    /// The rule covers literal components as well as `*` and `{a,b}`: a
    /// catalog pattern such as `.../User Data/*/Service Worker/CacheStorage`
    /// leaves `Service Worker` literal, and if that entry were a link the scan
    /// would otherwise read a tree the pattern never named. Nothing is deleted
    /// through such a path — the planner re-derives the authorizing root and
    /// refuses a link — but the *discovery* boundary is the same promise, and
    /// it is kept here.
    ///
    /// The rule applies **from the pattern's static root down**. The literal
    /// components before the first selector are the environment's own spelling
    /// of a root rather than a choice made here — macOS states `/var/folders/…`
    /// even though `/var` is itself a link into `/private/var` — so they are
    /// resolved as stated, and the no-indirection rule governs every component
    /// the pattern actually selects.
    ///
    /// The last component is the match itself, so it does not have to be a
    /// directory: an entry that exists but is a link is reported by the scan as
    /// a blocked observation instead of being silently skipped, and a missing
    /// one yields no match at all.
    pub fn expand(&self, flavor: PathFlavor, limit: usize) -> SelectionOutcome {
        let mut outcome = SelectionOutcome::default();
        // The frontier holds the path resolved so far. It starts at the
        // pattern's static root: joining the whole pattern would look for a
        // directory literally named `*`, and resolving the root's own literal
        // components through the traversal rule would refuse a platform root
        // the environment spelled with a link in it.
        let first_selector = self
            .components
            .iter()
            .position(|component| component.literal_name().is_none())
            .unwrap_or(self.components.len());
        let static_components: Vec<String> = self.components[..first_selector]
            .iter()
            .filter_map(|component| component.literal_name().map(str::to_string))
            .collect();
        if static_components.is_empty() {
            // A pattern that begins with a selector has no root to start from.
            // The manifest lint refuses one; an unmentioned one expands to
            // nothing rather than enumerating a filesystem root.
            return outcome;
        }

        let static_parts = PathParts {
            prefix: self.parts.prefix.clone(),
            rooted: self.parts.rooted,
            components: static_components,
        };
        let static_path = path_algebra::join_parts(&static_parts, flavor);
        let static_path_buf = PathBuf::from(&static_path);
        match std::fs::metadata(&static_path_buf) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return outcome;
            }
            Err(err) => {
                outcome.record_failure(static_path_buf, err.to_string());
                return outcome;
            }
        }

        let mut frontier = vec![static_parts];
        let mut truncated = false;

        let last_index = self.components.len().saturating_sub(1);
        for (index, component) in self.components.iter().enumerate().skip(first_selector) {
            let is_last = index == last_index;
            let mut next: Vec<PathParts> = Vec::new();
            for parts in &frontier {
                match component {
                    SelectorComponent::Literal(name) => {
                        let mut extended = parts.clone();
                        extended.components.push(name.clone());
                        let joined = path_algebra::join_parts(&extended, flavor);
                        let candidate_path = std::path::Path::new(&joined);
                        match evaluate_entry(candidate_path, is_last) {
                            EntryAcceptance::Accepted => {
                                next.push(extended);
                            }
                            EntryAcceptance::Rejected => {}
                            EntryAcceptance::Failed(err) => {
                                outcome.record_failure(candidate_path.to_path_buf(), err);
                            }
                        }
                    }
                    SelectorComponent::AnyOne | SelectorComponent::OneOf(_) => {
                        let base = path_algebra::join_parts(parts, flavor);
                        let base_path = std::path::Path::new(&base);
                        let entries = match std::fs::read_dir(base_path) {
                            Ok(entries) => entries,
                            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                            Err(err) => {
                                outcome.record_failure(base_path.to_path_buf(), err.to_string());
                                continue;
                            }
                        };
                        let mut names: Vec<String> = Vec::new();
                        for entry_result in entries {
                            let entry = match entry_result {
                                Ok(entry) => entry,
                                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                                Err(err) => {
                                    outcome
                                        .record_failure(base_path.to_path_buf(), err.to_string());
                                    continue;
                                }
                            };
                            let entry_path = entry.path();
                            let name = entry.file_name().to_string_lossy().into_owned();
                            if !component.matches(&name, flavor) {
                                continue;
                            }
                            match evaluate_entry(&entry_path, is_last) {
                                EntryAcceptance::Accepted => {
                                    names.push(name);
                                }
                                EntryAcceptance::Rejected => {}
                                EntryAcceptance::Failed(err) => {
                                    outcome.record_failure(entry_path, err);
                                }
                            }
                        }
                        names.sort_by(|left, right| {
                            path_algebra::fold(left, flavor)
                                .cmp(&path_algebra::fold(right, flavor))
                                .then_with(|| left.cmp(right))
                        });
                        for name in names {
                            if next.len() >= limit {
                                truncated = true;
                                break;
                            }
                            let mut extended = parts.clone();
                            extended.components.push(name);
                            next.push(extended);
                        }
                    }
                }
                if next.len() >= limit {
                    truncated = true;
                    break;
                }
            }
            if next.is_empty() {
                outcome.truncated = truncated;
                return outcome;
            }
            frontier = next;
        }

        let mut matches: Vec<PathBuf> = frontier
            .iter()
            .map(|parts| PathBuf::from(path_algebra::join_parts(parts, flavor)))
            .collect();
        matches.sort_by(|left, right| {
            path_algebra::fold(&left.to_string_lossy(), flavor)
                .cmp(&path_algebra::fold(&right.to_string_lossy(), flavor))
                .then_with(|| left.cmp(right))
        });
        outcome.matches = matches;
        outcome.truncated = truncated;
        outcome
    }

    /// Whether every match must be an existing directory (always true: the
    /// expansion only produces ones it saw).
    pub fn describes_directories(&self) -> bool {
        !self.components.is_empty()
    }
}

/// Replaces a `${...}` placeholder with a name-shaped marker.
///
/// Placeholder braces are not selector syntax, and a rule that confused the two
/// would refuse every `${LOCAL_APP_DATA}` path in the catalog. Masking keeps
/// the string's shape (separators and length), so the rules below see exactly
/// the characters the manifest author wrote as *pattern* syntax.
fn mask_placeholders(value: &str) -> String {
    let mut masked = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '$' && chars.peek() == Some(&'{') {
            chars.next();
            masked.push('$');
            masked.push('_');
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            masked.push('_');
        } else {
            masked.push(character);
        }
    }
    masked
}

enum EntryAcceptance {
    Accepted,
    Rejected,
    Failed(String),
}

/// Evaluates whether an entry may appear at this position of an expansion.
///
/// A component that is not the last one is descended into, so it must be a
/// directory the filesystem presents directly: a link, or a Windows reparse
/// point of any tag, ends the branch. The last component is the match, so it
/// only has to exist — the scan decides what it is and reports what it cannot
/// clean.
fn evaluate_entry(path: &Path, is_last: bool) -> EntryAcceptance {
    match std::fs::symlink_metadata(path) {
        // The last component is the match: it only has to exist, whatever it is.
        Ok(_) if is_last => EntryAcceptance::Accepted,
        Ok(metadata) if is_traversable_directory(&metadata) => EntryAcceptance::Accepted,
        Ok(_) => EntryAcceptance::Rejected,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => EntryAcceptance::Rejected,
        Err(err) => EntryAcceptance::Failed(err.to_string()),
    }
}

/// Whether metadata describes a directory the expansion may descend into.
///
/// On POSIX a `symlink_metadata` result is never a directory for a link, so the
/// type check is the whole rule. Windows reports a junction or mount point as a
/// directory *and* a reparse point, so the attribute is checked too: the
/// selector cannot ask which reparse tag it is from here, and refusing every
/// tag is the fail-closed direction.
#[cfg(windows)]
fn is_traversable_directory(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(not(windows))]
fn is_traversable_directory(metadata: &std::fs::Metadata) -> bool {
    metadata.is_dir()
}

/// Checks a pattern's selector syntax without a stated flavor.
///
/// The catalog loader validates before any environment exists, so the syntax
/// rules are expressed over both separator spellings: a malformed pattern is
/// refused wherever it would be written.
pub fn validate_pattern_syntax(pattern: &str) -> Result<(), String> {
    let masked = mask_placeholders(pattern);
    if !PatternSyntax::has_syntax(&masked) {
        return Ok(());
    }
    for component in masked.split(['/', '\\']) {
        if component.is_empty() || !PatternSyntax::has_syntax(component) {
            continue;
        }
        if component == "*" {
            continue;
        }
        let braced = component
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'));
        match braced {
            Some(inner)
                if !inner.is_empty()
                    && !inner.contains('{')
                    && !inner.contains('}')
                    && !inner.contains('*') =>
            {
                if inner.split(',').any(|name| name.trim().is_empty()) {
                    return Err(format!(
                        "`{pattern}`: a `{{...}}` component must not contain an empty name"
                    ));
                }
            }
            Some(_) => {
                return Err(format!(
                    "`{pattern}`: a `{{...}}` component may only list names, without nesting or wildcards"
                ));
            }
            None => {
                return Err(format!(
                    "`{pattern}`: a selector component must be exactly `*` or `{{a,b}}`; a partial wildcard is not supported"
                ));
            }
        }
    }
    Ok(())
}

struct PatternSyntax;

impl PatternSyntax {
    fn has_syntax(value: &str) -> bool {
        value.contains('*') || value.contains('{') || value.contains('}')
    }
}

/// Whether a value contains selector syntax that a catalog field forbids.
///
/// A `${...}` placeholder is a root, not a pattern, so its braces are ignored.
pub fn contains_selector_syntax(value: &str) -> bool {
    let masked = mask_placeholders(value);
    masked.contains('*') || masked.contains('{') || masked.contains('}')
}

/// Whether a path component is a platform's own indirection marker.
pub fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSIX: PathFlavor = PathFlavor::Posix;
    const WINDOWS: PathFlavor = PathFlavor::Windows;

    #[test]
    fn a_partial_wildcard_is_refused_rather_than_treated_as_a_literal() {
        let error = PathSelector::parse("/Users/me/Library/Caches/Cache*", POSIX)
            .expect_err("a partial wildcard is not a selector");
        assert!(error.reason.contains("partial wildcard"), "{error}");

        let nested = PathSelector::parse("/Users/me/{a,{b,c}}/x", POSIX)
            .expect_err("nested braces are refused");
        assert!(nested.reason.contains("nesting"), "{nested}");

        let traversal = PathSelector::parse("/Users/me/*/../x", POSIX)
            .expect_err("a catalog root never carries a parent traversal");
        assert!(traversal.reason.contains("`..`"), "{traversal}");

        let empty = PathSelector::parse("/Users/me/{}/x", POSIX)
            .expect_err("an empty alternative list is refused");
        assert!(empty.reason.contains("at least one name"), "{empty}");
    }

    /// A placeholder is a root, not a pattern: the catalog names roots with
    /// `${...}` and selectors with `*`/`{...}`, and the two must not be
    /// confused in either direction.
    #[test]
    fn placeholders_are_not_selector_syntax() {
        assert!(!contains_selector_syntax(
            "${LOCAL_APP_DATA}/Cursor/User/settings.json"
        ));
        assert!(contains_selector_syntax(
            "${LOCAL_APP_DATA}/Packages/*/TempState"
        ));
        assert!(contains_selector_syntax("${LOCAL_APP_DATA}/Packages/{A,B}"));
        assert!(!contains_selector_syntax(
            "~/Library/Caches/com.apple.Safari"
        ));

        assert!(validate_pattern_syntax("${LOCAL_APP_DATA}/Cursor/User/settings.json").is_ok());
        assert!(
            validate_pattern_syntax("${LOCAL_APP_DATA}/Packages/*/{LocalCache,TempState}").is_ok()
        );
        assert!(validate_pattern_syntax("${LOCAL_APP_DATA}/Packages/Cache*").is_err());
    }

    #[test]
    fn matching_follows_the_stated_flavor() {
        let selector = PathSelector::parse(
            r"C:\Users\me\AppData\Local\Packages\*\{LocalCache,TempState}",
            WINDOWS,
        )
        .expect("the pattern parses");
        assert!(selector.matches(
            r"C:\Users\me\AppData\Local\Packages\Microsoft.WindowsCalculator_8wekyb3d8bbwe\TempState",
            WINDOWS
        ));
        assert!(selector.matches(
            r"c:\users\ME\appdata\local\packages\microsoft.windowscalculator_8wekyb3d8bbwe\localcache",
            WINDOWS
        ));
        assert!(!selector.matches(
            r"C:\Users\me\AppData\Local\Packages\Microsoft.WindowsCalculator_8wekyb3d8bbwe\LocalState",
            WINDOWS
        ));
        assert!(!selector.matches(
            // One component too few: `TempState` was matched by the brace, and
            // the `*` would have to be the package name.
            r"C:\Users\me\AppData\Local\Packages\TempState",
            WINDOWS
        ));

        let posix = PathSelector::parse("/Users/me/Library/Application Support/*/GPUCache", POSIX)
            .expect("the pattern parses");
        assert!(posix.matches(
            "/Users/me/Library/Application Support/Slack/GPUCache",
            POSIX
        ));
        assert!(!posix.matches(
            "/Users/me/Library/Application Support/slack/gpucache",
            POSIX
        ));
    }

    #[test]
    fn the_static_root_and_match_key_describe_what_the_pattern_left_open() {
        let selector =
            PathSelector::parse("/Users/me/Library/Application Support/*/GPUCache", POSIX)
                .expect("the pattern parses");
        assert_eq!(
            selector.static_root().as_deref(),
            Some("/Users/me/Library/Application Support")
        );
        assert_eq!(
            selector
                .match_key(
                    "/Users/me/Library/Application Support/Slack/GPUCache",
                    POSIX
                )
                .as_deref(),
            Some("Slack")
        );
        // Two matches of one pattern never share a key.
        assert_ne!(
            selector.match_key(
                "/Users/me/Library/Application Support/Slack/GPUCache",
                POSIX
            ),
            selector.match_key(
                "/Users/me/Library/Application Support/Discord/GPUCache",
                POSIX
            )
        );
    }

    #[test]
    fn expansion_visits_only_real_directories_in_a_deterministic_order() {
        let fixture = tempfile::tempdir().expect("fixture");
        let support = fixture.path().join("Application Support");
        for app in ["Slack", "Discord", "zoom.us"] {
            std::fs::create_dir_all(support.join(app).join("GPUCache")).expect("app fixture");
            std::fs::create_dir_all(support.join(app).join("Code Cache")).expect("app fixture");
        }
        // A file named like an application is not a root, and neither is a
        // link to one.
        std::fs::write(support.join("Loose.app"), b"not a directory").expect("file fixture");
        #[cfg(unix)]
        std::os::unix::fs::symlink(support.join("Slack"), support.join("Slack Link"))
            .expect("link fixture");

        let pattern = format!("{}/*/GPUCache", support.to_string_lossy());
        let selector = PathSelector::parse(&pattern, POSIX).expect("the pattern parses");
        let outcome = selector.expand(POSIX, SELECTION_LIMIT);

        let names: Vec<String> = outcome
            .matches
            .iter()
            .map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(names, vec!["GPUCache", "GPUCache", "GPUCache"]);
        assert!(!outcome.truncated);
        let parents: Vec<String> = outcome
            .matches
            .iter()
            .map(|path| {
                path.parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(parents, vec!["Discord", "Slack", "zoom.us"]);
    }

    /// A link in the middle of a pattern ends that branch: the expansion must
    /// not read a tree the catalog never named, even though nothing would be
    /// deleted through it.
    #[cfg(unix)]
    #[test]
    fn a_linked_component_is_never_descended_into() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().expect("fixture");
        let app = fixture.path().join("Profile 1");
        std::fs::create_dir_all(app.join("Service Worker").join("CacheStorage")).expect("fixture");
        std::fs::write(app.join("Service Worker/CacheStorage/blob"), b"own").expect("fixture");

        // The same location, reached only through a link, holds another tree.
        let outside = fixture.path().join("outside");
        std::fs::create_dir_all(outside.join("CacheStorage")).expect("fixture");
        std::fs::write(outside.join("CacheStorage/blob"), b"foreign").expect("fixture");

        let pattern = format!(
            "{}/*/Service Worker/CacheStorage",
            fixture.path().to_string_lossy()
        );
        let selector = PathSelector::parse(&pattern, POSIX).expect("the pattern parses");

        let direct = selector.expand(POSIX, SELECTOR_MATCH_LIMIT);
        assert_eq!(direct.matches.len(), 1, "the real subtree matches");
        assert!(direct.matches[0].starts_with(fixture.path()));

        // Now the literal component is a link to the other tree.
        let linked = fixture.path().join("Profile 2");
        std::fs::create_dir_all(&linked).expect("fixture");
        symlink(outside.join("CacheStorage"), linked.join("Service Worker")).expect("link fixture");
        // ... and a link to the cache directory itself, one level up.
        let linked_parent = fixture.path().join("Profile 3");
        std::fs::create_dir_all(&linked_parent).expect("fixture");
        symlink(&outside, linked_parent.join("Service Worker")).expect("link fixture");

        let outcome = selector.expand(POSIX, SELECTOR_MATCH_LIMIT);
        assert_eq!(
            outcome.matches.len(),
            1,
            "a linked component is not a match: {outcome:?}"
        );
        assert!(
            outcome.matches[0].starts_with(fixture.path()),
            "the tree behind the link is never read"
        );
    }

    /// The last component is the match itself: an entry that exists but is a
    /// link is still reported, so the scan can say why it is not cleanable.
    #[cfg(unix)]
    #[test]
    fn a_linked_last_component_is_matched_for_the_scan_to_classify() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().expect("fixture");
        let app = fixture.path().join("com.example.app");
        std::fs::create_dir_all(&app).expect("fixture");
        let cache = fixture.path().join("elsewhere");
        std::fs::create_dir_all(&cache).expect("fixture");
        symlink(&cache, app.join("GPUCache")).expect("link fixture");

        let pattern = format!("{}/*/GPUCache", fixture.path().to_string_lossy());
        let selector = PathSelector::parse(&pattern, POSIX).expect("the pattern parses");
        let outcome = selector.expand(POSIX, SELECTOR_MATCH_LIMIT);
        assert_eq!(outcome.matches.len(), 1);
        assert!(outcome.matches[0].ends_with("GPUCache"));
    }

    #[test]
    fn a_capped_expansion_reports_the_truncation() {
        let fixture = tempfile::tempdir().expect("fixture");
        for index in 0..4 {
            std::fs::create_dir_all(fixture.path().join(format!("app{index}"))).expect("fixture");
        }
        let pattern = format!("{}/*", fixture.path().to_string_lossy());
        let selector = PathSelector::parse(&pattern, POSIX).expect("the pattern parses");

        let outcome = selector.expand(POSIX, 2);
        assert_eq!(outcome.matches.len(), 2);
        assert!(outcome.truncated);

        let outcome = selector.expand(POSIX, SELECTION_LIMIT);
        assert_eq!(outcome.matches.len(), 4);
        assert!(!outcome.truncated);
    }

    #[test]
    fn a_pattern_that_matches_nothing_is_an_empty_selection() {
        let fixture = tempfile::tempdir().expect("fixture");
        let pattern = format!("{}/*/absent", fixture.path().to_string_lossy());
        let selector = PathSelector::parse(&pattern, POSIX).expect("the pattern parses");
        let outcome = selector.expand(POSIX, SELECTION_LIMIT);
        assert!(outcome.matches.is_empty());
        assert!(!outcome.truncated);
        assert!(outcome.failures.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn an_inaccessible_intermediate_directory_reports_selection_failure() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("Containers");
        let app = parent.join("com.example.app");
        let cache = app.join("Data").join("Caches");
        std::fs::create_dir_all(&cache).expect("fixture");

        // Make Containers unreadable
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o000)).expect("chmod");

        let pattern = format!(
            "{}/Containers/*/Data/Caches",
            fixture.path().to_string_lossy()
        );
        let selector = PathSelector::parse(&pattern, POSIX).expect("the pattern parses");
        let outcome = selector.expand(POSIX, SELECTOR_MATCH_LIMIT);

        // Restore permission so tempdir cleanup succeeds
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        assert!(outcome.matches.is_empty());
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].path, parent);
    }

    const SELECTION_LIMIT: usize = SELECTOR_MATCH_LIMIT;
}
