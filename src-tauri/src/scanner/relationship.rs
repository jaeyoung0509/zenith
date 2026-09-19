//! Establishing which two discovered units describe the same location.
//!
//! The scan and the planner both need this answer, and they must not derive it
//! differently: a scan states what it found, and a plan authorizes a subset of
//! what that scan reported. Deriving it from the OS family instead — "Windows
//! folds case, POSIX does not" — gets it wrong in both directions, because case
//! behaviour is a property of the volume and not of the platform: a Windows
//! volume with per-directory case sensitivity holds two distinct entries, and a
//! case-folding APFS volume holds one.
//!
//! The objects answer first: stable filesystem identity decides whether two
//! units are the same entry, and the identities of the ancestor entries decide
//! containment. Only when an identity cannot be obtained does the comparison
//! fall back to path text, conservatively case-sensitive.

use crate::models::{FileIdentity, PathIdentity, ScanItem, UnitRelationship};
use std::ffi::OsString;
use std::path::Path;

/// Resolve the actual directory entry for a spelling that reached `entity`.
/// On a case-folding volume, `pip` can open an entry stored as `Pip`; on a
/// case-sensitive volume both spellings may be separate hardlinks to one inode.
pub(crate) fn actual_entry_name(path: &Path, entity: FileIdentity) -> Option<OsString> {
    let requested = path.file_name()?;
    let mut folded_match = None;
    for entry in std::fs::read_dir(path.parent()?).ok()?.flatten() {
        let name = entry.file_name();
        if name != requested
            && !name
                .to_string_lossy()
                .eq_ignore_ascii_case(&requested.to_string_lossy())
        {
            continue;
        }
        let matches_entity = crate::safety::ToctouGuard::capture(&entry.path())
            .is_some_and(|identity| identity.entity() == entity);
        if !matches_entity {
            continue;
        }
        if name == requested {
            return Some(name);
        }
        if folded_match.is_some() {
            return None;
        }
        folded_match = Some(name);
    }
    folded_match
}

/// Whether both paths name one directory entry, whatever their spelling.
pub(crate) fn same_directory_entry(first: &Path, second: &Path, entity: FileIdentity) -> bool {
    let parents_match = first
        .parent()
        .and_then(crate::safety::ToctouGuard::capture)
        .zip(
            second
                .parent()
                .and_then(crate::safety::ToctouGuard::capture),
        )
        .is_some_and(|(left, right)| left.entity().same_entity(right.entity()));
    parents_match
        && actual_entry_name(first, entity)
            .zip(actual_entry_name(second, entity))
            .is_some_and(|(left, right)| left == right)
}

/// The relationship the filesystem establishes between two units, without
/// policy: whether they name one object, one contains the other, or they are
/// separate locations.
pub(crate) fn unit_relationship(candidate: &ScanItem, container: &ScanItem) -> UnitRelationship {
    if let Some(relationship) = filesystem_relationship(candidate, container) {
        return relationship;
    }
    // No stable identity is available (the path is gone, or the platform
    // cannot state one). Text is the conservative fallback: case-sensitive, so
    // two spellings are treated as two locations rather than one.
    let candidate_key = candidate.unit_identity(PathIdentity::CaseSensitive);
    let container_key = container.unit_identity(PathIdentity::CaseSensitive);
    if candidate_key == container_key {
        UnitRelationship::Equivalent
    } else if candidate_key.is_within(&container_key) {
        UnitRelationship::Contained
    } else {
        UnitRelationship::Distinct
    }
}

/// The relationship stable filesystem identity establishes, when it can.
fn filesystem_relationship(candidate: &ScanItem, container: &ScanItem) -> Option<UnitRelationship> {
    let candidate_path = Path::new(&candidate.unit.path);
    let container_path = Path::new(&container.unit.path);
    let candidate_identity = crate::safety::ToctouGuard::capture(candidate_path)?;
    let container_identity = crate::safety::ToctouGuard::capture(container_path)?;
    let candidate_entity = candidate_identity.entity();
    let container_entity = container_identity.entity();
    if candidate_entity.is_unknown() || container_entity.is_unknown() {
        return None;
    }
    if candidate_entity.same_entity(container_entity)
        && same_directory_entry(candidate_path, container_path, candidate_entity)
    {
        return Some(UnitRelationship::Equivalent);
    }
    if candidate_path.ancestors().skip(1).any(|ancestor| {
        crate::safety::ToctouGuard::capture(ancestor)
            .is_some_and(|identity| identity.entity().same_entity(container_entity))
    }) {
        return Some(UnitRelationship::Contained);
    }
    Some(UnitRelationship::Distinct)
}

#[cfg(test)]
mod tests {
    use super::unit_relationship;
    use crate::models::{Category, CleanupUnit, FileSize, RiskTier, ScanItem, UnitRelationship};

    fn item(signature: &str, path: &str) -> ScanItem {
        ScanItem::mock(
            format!("{signature}:{path}"),
            signature,
            signature,
            Category::System,
            RiskTier::Safe,
            path,
            FileSize::new(1_024, Some(1_024)),
            1,
        )
    }

    /// Identity decides first: two spellings that reach the same directory
    /// entry are one unit, whatever their text.
    #[test]
    fn stable_identity_finds_one_entry_under_two_spellings() {
        let fixture = tempfile::tempdir().expect("fixture");
        let stored = fixture.path().join("Cache");
        std::fs::create_dir(&stored).expect("fixture");
        let alternate = fixture.path().join("cache");

        let candidate = item("test.stored", &stored.to_string_lossy());
        let container = item("test.alternate", &alternate.to_string_lossy());

        if alternate.exists() {
            // A folding volume answers to both spellings: one entry, one unit.
            assert_eq!(
                unit_relationship(&candidate, &container),
                UnitRelationship::Equivalent
            );
        } else {
            // A case-sensitive volume holds one entry under one spelling: the
            // missing one is not that entry.
            assert_eq!(
                unit_relationship(&candidate, &container),
                UnitRelationship::Distinct
            );
        }
    }

    /// Containment is established by the identities of the actual ancestor
    /// entries, not by string prefixes: a sibling whose name merely starts the
    /// same way is not inside.
    #[test]
    fn containment_follows_the_real_ancestors() {
        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("Cache");
        let child = parent.join("nested");
        let sibling = fixture.path().join("CacheX");
        std::fs::create_dir_all(&child).expect("fixture");
        std::fs::create_dir_all(&sibling).expect("fixture");

        let parent_item = item("test.parent", &parent.to_string_lossy());
        let child_item = item("test.child", &child.to_string_lossy());
        let sibling_item = item("test.sibling", &sibling.to_string_lossy());

        assert_eq!(
            unit_relationship(&child_item, &parent_item),
            UnitRelationship::Contained
        );
        assert_eq!(
            unit_relationship(&sibling_item, &parent_item),
            UnitRelationship::Distinct,
            "a shared name prefix is not containment"
        );
    }

    /// When no stable identity can be obtained, the fallback is text, and it is
    /// conservative: two spellings are two locations rather than one.
    #[test]
    fn missing_identity_falls_back_to_case_sensitive_text() {
        let upper = item("test.upper", r"C:\work\Cache");
        let lower = item("test.lower", r"C:\work\cache");
        let nested = item("test.nested", r"C:\work\Cache\nested");

        assert_eq!(
            unit_relationship(&upper, &lower),
            UnitRelationship::Distinct,
            "an unavailable identity does not fold two spellings into one"
        );
        assert_eq!(
            unit_relationship(&nested, &upper),
            UnitRelationship::Contained
        );
    }

    /// The unit kind does not change the structural answer: this function
    /// states what the filesystem shows, and policy is decided by the caller.
    #[test]
    fn the_structural_answer_ignores_authority() {
        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("Cache");
        let child = parent.join("nested");
        std::fs::create_dir_all(&child).expect("fixture");

        let mut child_item = item("test.child", &child.to_string_lossy());
        child_item.unit = CleanupUnit::new(
            crate::models::CleanupUnitKind::ProviderAction,
            child.to_string_lossy().into_owned(),
            child.to_string_lossy().into_owned(),
        );
        let parent_item = item("test.parent", &parent.to_string_lossy());

        assert_eq!(
            unit_relationship(&child_item, &parent_item),
            UnitRelationship::Contained
        );
    }
}
