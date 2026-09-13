//! Filesystem identity: which entity on which volume, and what each domain is
//! allowed to conclude from it.
//!
//! Zenith mutates files on two independent paths — generic cleanup, which is
//! authorized by a backend-owned [`crate::domain::cleanup::DeletePlan`], and
//! the reviewed storage workflows (Large Files, Applications, Developer
//! Artifacts, Trash), which are authorized by an explicit user selection.
//! Both need to answer "is this still the same file?", but they capture and
//! compare different evidence, and one must never stand in for the other.
//!
//! [`FileIdentity`] is the one fact they agree on: the device and inode a path
//! resolved to. Everything else — whether the entry is a directory, how many
//! bytes it held, when it was last written, and whether a zero reading is
//! permissible — is captured by the authority that asked for it:
//! [`CleanupIdentity`] for generic cleanup, [`ReviewedFileIdentity`] for a
//! user-reviewed target. The wrapper types are what make a reviewed Trash
//! target structurally unable to satisfy a cleanup TOCTOU check.

use std::fmt;

/// The entity a path resolved to: a device and an inode.
///
/// `(0, 0)` is the platform saying "no stable identity could be derived" —
/// FAT32 and some network shares on Windows, and any platform without an
/// equivalent API. It is never a match. A caller that treats it as one would
/// fail open, so [`FileIdentity::is_unknown`] exists to force that decision
/// into the code rather than into a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    pub const UNKNOWN: Self = Self {
        device: 0,
        inode: 0,
    };

    pub const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    pub const fn device(self) -> u64 {
        self.device
    }

    pub const fn inode(self) -> u64 {
        self.inode
    }

    /// Whether the platform failed to derive an identity.
    pub const fn is_unknown(self) -> bool {
        self.device == 0 && self.inode == 0
    }

    /// Whether two captures name the same entity.
    ///
    /// Unknown identities never match, in either direction.
    pub fn same_entity(self, other: Self) -> bool {
        !self.is_unknown() && !other.is_unknown() && self == other
    }
}

impl fmt::Display for FileIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dev={}, ino={}", self.device, self.inode)
    }
}

/// A modification stamp with sub-second resolution.
///
/// Seconds and nanoseconds are one value. Comparing only the seconds of two
/// stamps accepts a file that was rewritten within the same second, which is
/// exactly the rewrite a TOCTOU check exists to catch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ModifiedStamp {
    secs: u64,
    nanos: u32,
}

impl ModifiedStamp {
    pub const fn new(secs: u64, nanos: u32) -> Self {
        Self { secs, nanos }
    }

    pub const fn secs(self) -> u64 {
        self.secs
    }

    pub const fn nanos(self) -> u32 {
        self.nanos
    }
}

impl fmt::Display for ModifiedStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:09}", self.secs, self.nanos)
    }
}

/// What generic cleanup captured about a target when its plan was built.
///
/// The executor re-captures the same facts immediately before mutating and
/// requires every one of them to agree: the entity, the file type, the
/// modification stamp, and — for files only — the size. Directories are
/// freshness-checked by their own mtime; stale-temp signatures re-measure the
/// whole tree instead, because a directory's mtime does not move when a file
/// deep inside it is rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupIdentity {
    entity: FileIdentity,
    is_dir: bool,
    size: u64,
    modified: ModifiedStamp,
}

impl CleanupIdentity {
    pub const fn new(
        entity: FileIdentity,
        is_dir: bool,
        size: u64,
        modified: ModifiedStamp,
    ) -> Self {
        Self {
            entity,
            is_dir,
            size,
            modified,
        }
    }

    pub const fn entity(self) -> FileIdentity {
        self.entity
    }

    pub const fn is_dir(self) -> bool {
        self.is_dir
    }

    pub const fn size(self) -> u64 {
        self.size
    }

    pub const fn modified(self) -> ModifiedStamp {
        self.modified
    }

    /// Whether a re-captured identity names the same entity as this one.
    pub fn same_entity(self, other: Self) -> bool {
        self.entity.same_entity(other.entity)
    }
}

/// What a reviewed storage workflow captured about a target the user selected.
///
/// The review that authorized the mutation happened in an earlier pass, so
/// this identity is checked twice: once when the target is revalidated against
/// the inventory it came from, and once immediately before the move. Full
/// equality — entity, size, and mtime — is the default comparison because a
/// reviewed file that grew or was rewritten is no longer the file the user
/// approved. Directory anchors compare with [`Self::same_entity`] instead: a
/// workspace directory gains and loses entries while the review is open, and
/// only the directory's own identity is stable across that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewedFileIdentity {
    entity: FileIdentity,
    size: u64,
    /// Modification time in whole seconds. `None` when the platform reported
    /// no usable timestamp, which is a weaker identity, not a missing one.
    modified: Option<u64>,
}

impl ReviewedFileIdentity {
    pub const fn new(entity: FileIdentity, size: u64, modified: Option<u64>) -> Self {
        Self {
            entity,
            size,
            modified,
        }
    }

    pub const fn entity(&self) -> FileIdentity {
        self.entity
    }

    pub const fn device(&self) -> u64 {
        self.entity.device()
    }

    pub const fn inode(&self) -> u64 {
        self.entity.inode()
    }

    pub const fn size(&self) -> u64 {
        self.size
    }

    pub const fn modified(&self) -> Option<u64> {
        self.modified
    }

    pub const fn is_unknown(&self) -> bool {
        self.entity.is_unknown()
    }

    /// Whether two captures name the same entity, ignoring freshness.
    pub fn same_entity(&self, other: &Self) -> bool {
        self.entity.same_entity(other.entity)
    }

    pub const fn with_size(&self, size: u64) -> Self {
        Self {
            entity: self.entity,
            size,
            modified: self.modified,
        }
    }

    pub const fn with_modified(&self, modified: Option<u64>) -> Self {
        Self {
            entity: self.entity,
            size: self.size,
            modified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_identity_never_matches_another() {
        let unknown = FileIdentity::UNKNOWN;
        assert!(unknown.is_unknown());
        assert!(!unknown.same_entity(unknown));
        assert!(!unknown.same_entity(FileIdentity::new(1, 1)));
        assert!(!FileIdentity::new(1, 1).same_entity(unknown));
    }

    #[test]
    fn a_zero_device_with_a_real_inode_is_still_a_usable_identity() {
        let identity = FileIdentity::new(0, 42);
        assert!(!identity.is_unknown());
        assert!(identity.same_entity(FileIdentity::new(0, 42)));
    }

    #[test]
    fn same_entity_ignores_size_and_mtime_but_equality_does_not() {
        let original = ReviewedFileIdentity::new(FileIdentity::new(3, 7), 10, Some(100));
        let rewritten = ReviewedFileIdentity::new(FileIdentity::new(3, 7), 99, Some(200));

        assert!(original.same_entity(&rewritten));
        assert_ne!(original, rewritten);
    }

    #[test]
    fn cleanup_identity_agrees_only_on_the_shared_entity() {
        let first =
            CleanupIdentity::new(FileIdentity::new(5, 9), false, 1, ModifiedStamp::new(1, 2));
        let same_entity_different_evidence =
            CleanupIdentity::new(FileIdentity::new(5, 9), true, 500, ModifiedStamp::new(9, 9));

        assert!(first.same_entity(same_entity_different_evidence));
        assert_ne!(first, same_entity_different_evidence);
    }

    #[test]
    fn a_sub_second_rewrite_changes_the_stamp() {
        assert_ne!(
            ModifiedStamp::new(1_700_000_000, 0),
            ModifiedStamp::new(1_700_000_000, 1)
        );
        assert!(ModifiedStamp::new(1_700_000_000, 0) < ModifiedStamp::new(1_700_000_001, 0));
    }
}
