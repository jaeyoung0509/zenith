//! Cleanup units: what a discovered candidate *is*, and how two candidates are
//! told apart.
//!
//! A scan for a broad root (`~/Library/Caches`, `$TMPDIR`) does not discover
//! one thing: it discovers many independently owned children, each with its own
//! age, owner, and eligibility. [`CleanupUnit`] is the name for that unit of
//! ownership, and it is what the planner, the execution guard, and the
//! accounting layer all agree on.
//!
//! The unit is deliberately not the same thing as the configured root. A
//! signature declares a root and a [`CleanupUnitKind`]; the kind says whether
//! the root itself is the deletable object (`FixedPath`) or whether each child
//! is (`ChildNamespace`). Deleting a root because a child of it was stale is
//! exactly the mistake the distinction prevents.
//!
//! Identity is path-shaped and normalized here rather than by the caller,
//! because two signatures can name the same directory with different casing and
//! separators, and a scan that counted it twice would report a total no user
//! could reconcile. [`PathIdentity`] states the one platform fact that
//! normalization cannot derive in a shared type: whether the filesystem folds
//! case.

use serde::{Deserialize, Serialize};

/// What kind of object a signature's cleanup unit is.
///
/// The kind decides where the deletable boundary sits: a `FixedPath` signature
/// authorizes the configured path, a `ChildNamespace` signature authorizes the
/// children it enumerated under that path, and the two non-filesystem kinds
/// authorize no host path at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CleanupUnitKind {
    /// The configured path is itself the deletable object.
    #[default]
    FixedPath,
    /// Each direct child of the configured root is a deletable object.
    ChildNamespace,
    /// A named disposable subtree inside an application-owned directory.
    NamedSubtree,
    /// No host path: a provider CLI invalidates its own cache with fixed
    /// arguments, and the path is only a staleness assertion.
    ProviderAction,
    /// No host path: a container runtime prunes the resources it owns.
    ContainerResource,
}

impl CleanupUnitKind {
    /// Whether this kind authorizes a filesystem mutation.
    pub fn is_filesystem(&self) -> bool {
        matches!(
            self,
            Self::FixedPath | Self::ChildNamespace | Self::NamedSubtree
        )
    }

    /// Whether the unit is a child the scanner enumerated under a root, so a
    /// candidate path at the root's own level is not a valid member.
    pub fn is_enumerated_child(&self) -> bool {
        matches!(self, Self::ChildNamespace)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::FixedPath => "Configured path",
            Self::ChildNamespace => "Enumerated child",
            Self::NamedSubtree => "Named cache subtree",
            Self::ProviderAction => "Provider action",
            Self::ContainerResource => "Container resource",
        }
    }
}

/// One deletable object, with the root it was discovered under.
///
/// `path` is where the unit lives; `root` is the configured path the signature
/// resolved. Keeping both means the execution guard can re-assert containment
/// ("this is still a child of the root that authorized it") instead of trusting
/// a path string the plan carried.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct CleanupUnit {
    pub kind: CleanupUnitKind,
    pub root: String,
    pub path: String,
}

impl CleanupUnit {
    pub fn new(kind: CleanupUnitKind, root: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind,
            root: root.into(),
            path: path.into(),
        }
    }

    /// The configured path itself is the unit.
    pub fn fixed_path(path: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            kind: CleanupUnitKind::FixedPath,
            root: path.clone(),
            path,
        }
    }

    /// Each enumerated child of `root` is its own unit.
    pub fn child_namespace(root: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            kind: CleanupUnitKind::ChildNamespace,
            root: root.into(),
            path: path.into(),
        }
    }

    /// Whether this unit names a path a mutation could touch.
    ///
    /// A default-constructed unit (an older scan, a deserialized payload) is
    /// not declared, and the planner refuses it rather than treating an empty
    /// path as a target.
    pub fn is_declared(&self) -> bool {
        !self.path.is_empty() && !self.root.is_empty()
    }

    /// The identity two discoveries of the same object must share.
    ///
    /// Normalization is textual and platform-stated: separators are unified,
    /// a trailing separator is dropped, and case is folded only when the
    /// filesystem the scan ran against folds it. Two signatures that name the
    /// same cache directory therefore produce one key, and the aggregation
    /// layer can count the bytes once.
    pub fn identity(&self, identity: PathIdentity) -> CleanupUnitIdentity {
        let mut normalized = String::with_capacity(self.path.len());
        for component in self.path.split(['/', '\\']) {
            if component.is_empty() {
                continue;
            }
            if !normalized.is_empty() {
                normalized.push('/');
            }
            if identity == PathIdentity::CaseInsensitive {
                normalized.extend(component.chars().flat_map(char::to_lowercase));
            } else {
                normalized.push_str(component);
            }
        }
        CleanupUnitIdentity(normalized)
    }
}

/// The normalized key that identifies one unit within a scan.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CleanupUnitIdentity(String);

impl CleanupUnitIdentity {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CleanupUnitIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether the filesystem the scan observed folds case in path comparisons.
///
/// The caller states the fact. Path syntax alone does not determine case
/// behavior: APFS and NTFS volumes can each use different settings. The
/// scanner compares stable filesystem identities for existing objects and
/// uses case-sensitive text only when an identity is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathIdentity {
    #[default]
    CaseSensitive,
    CaseInsensitive,
}

/// Who is expected to own a discovered unit, and how strongly that is known.
///
/// Ownership is a claim, not a measurement, so it carries its own confidence.
/// A catalog entry that names an owner states it; an entry that only names the
/// provider has it inferred from that provider; everything else is unknown, and
/// a message built from an unknown owner must not pretend otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipConfidence {
    /// The catalog states the owner of this location.
    Declared,
    /// The owner is inferred from the provider the catalog named.
    Inferred,
    /// Nothing in the catalog identifies the owner.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
pub struct CleanupOwnership {
    pub owner: String,
    pub confidence: OwnershipConfidence,
}

impl CleanupOwnership {
    /// The catalog named the owner of this location.
    pub fn declared(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            confidence: OwnershipConfidence::Declared,
        }
    }

    /// Only the provider was named, so the owner is inferred from it.
    pub fn inferred(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            confidence: OwnershipConfidence::Inferred,
        }
    }

    pub fn unknown() -> Self {
        Self::default()
    }

    /// Whether an owner can be named at all.
    pub fn is_known(&self) -> bool {
        !self.owner.is_empty() && self.confidence != OwnershipConfidence::Unknown
    }
}

/// What the age policy observed about a unit, and whether it is satisfied.
///
/// The whole-tree newest timestamp is the fact, and the comparison against the
/// policy is recorded with it because it happened at a stated instant that a
/// later reader does not have. Construction goes through [`Self::evaluate`] so
/// the two can never disagree, and a unit whose newest timestamp could not be
/// read is never satisfied: an unprovable age is not an old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgeObservation {
    pub min_age_days: u32,
    #[serde(default, with = "crate::ipc_numeric::option_u64")]
    #[specta(type = Option<u64>)]
    pub newest_modified: Option<u64>,
    pub satisfied: bool,
}

impl AgeObservation {
    /// Evaluates an age policy against the newest timestamp a measurement saw.
    pub fn evaluate(min_age_days: u32, newest_modified: Option<u64>, now: u64) -> Self {
        let satisfied = match newest_modified {
            Some(newest) => now
                .checked_sub(newest)
                .is_some_and(|age| age >= u64::from(min_age_days) * 86_400),
            None => false,
        };
        Self {
            min_age_days,
            newest_modified,
            satisfied,
        }
    }

    /// The age in seconds the newest entry in the unit had, when it was read.
    pub fn age_seconds(&self, now: u64) -> Option<u64> {
        self.newest_modified
            .and_then(|newest| now.checked_sub(newest))
    }
}

/// The scan-policy fact that decides whether a discovered unit may be cleaned.
///
/// Discovery and deletion permission are separate concerns: a scan can honestly
/// inventory storage that the current settings refuse to clean. Recording the
/// gate as a fact on the item (rather than filtering the item out at discovery
/// time) is what lets the interface say "found this, it is not eligible here,
/// and here is why".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EligibilityGate {
    /// The current settings permit cleaning a unit this signature discovered.
    #[default]
    Open,
    /// The signature is opt-in, and the opt-in is off in the current settings.
    IntensiveCleanupDisabled,
}

impl EligibilityGate {
    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }

    /// The reason a gated unit is not cleanable, phrased for the interface.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Open => None,
            Self::IntensiveCleanupDisabled => {
                Some("Discovered for visibility only: intensive cleanup is disabled in settings")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_identity_unifies_separators_and_trailing_slashes() {
        let posix = CleanupUnit::fixed_path("/Users/tester/Library/Caches/pip");
        let trailing = CleanupUnit::fixed_path("/Users/tester/Library/Caches/pip/");
        let windows_shaped = CleanupUnit::fixed_path(r"\Users\tester\Library\Caches\pip");

        let key = |unit: &CleanupUnit| unit.identity(PathIdentity::CaseSensitive);
        assert_eq!(key(&posix), key(&trailing));
        assert_eq!(key(&posix), key(&windows_shaped));
        assert_eq!(key(&posix).as_str(), "Users/tester/Library/Caches/pip");
    }

    /// A case-insensitive filesystem must not report the same directory twice,
    /// and a case-sensitive one must not fold two different directories into
    /// one.
    #[test]
    fn case_folding_follows_the_stated_filesystem() {
        let upper = CleanupUnit::fixed_path("/Users/tester/Library/Caches/Pip");
        let lower = CleanupUnit::fixed_path("/users/tester/library/caches/pip");

        assert_ne!(
            upper.identity(PathIdentity::CaseSensitive),
            lower.identity(PathIdentity::CaseSensitive)
        );
        assert_eq!(
            upper.identity(PathIdentity::CaseInsensitive),
            lower.identity(PathIdentity::CaseInsensitive)
        );
    }

    #[test]
    fn an_undeclared_unit_is_not_a_target() {
        assert!(!CleanupUnit::default().is_declared());
        assert!(CleanupUnit::fixed_path("/tmp/cache").is_declared());
        assert!(CleanupUnit::child_namespace("/tmp", "/tmp/cache").is_declared());
    }

    /// The gate is the only reason a discovered unit may be ineligible without
    /// being either unsafe or empty.
    #[test]
    fn the_intensive_gate_names_its_reason() {
        assert!(EligibilityGate::Open.is_open());
        assert!(EligibilityGate::Open.reason().is_none());
        let gated = EligibilityGate::IntensiveCleanupDisabled;
        assert!(!gated.is_open());
        assert!(gated.reason().unwrap().contains("intensive cleanup"));
    }

    #[test]
    fn age_is_only_satisfied_when_it_is_provable() {
        let now = 2_000_000_000;
        let ten_days = 10 * 86_400;

        let stale = AgeObservation::evaluate(7, Some(now - ten_days), now);
        assert!(stale.satisfied);
        assert_eq!(stale.age_seconds(now), Some(ten_days));

        let fresh = AgeObservation::evaluate(7, Some(now - 86_400), now);
        assert!(!fresh.satisfied);

        // Exactly at the threshold counts as old enough; one second less does not.
        assert!(AgeObservation::evaluate(7, Some(now - 7 * 86_400), now).satisfied);
        assert!(!AgeObservation::evaluate(7, Some(now - 7 * 86_400 + 1), now).satisfied);

        // An unreadable timestamp and a clock that moved backwards both fail
        // closed rather than reading as "old enough".
        assert!(!AgeObservation::evaluate(7, None, now).satisfied);
        assert!(!AgeObservation::evaluate(7, Some(now + 60), now).satisfied);
    }

    #[test]
    fn ownership_states_whether_the_owner_is_known() {
        assert!(!CleanupOwnership::unknown().is_known());
        assert!(CleanupOwnership::declared("Playwright CLI").is_known());
        let inferred = CleanupOwnership::inferred("PyTorch / Triton");
        assert!(inferred.is_known());
        assert_eq!(inferred.confidence, OwnershipConfidence::Inferred);
    }
}
