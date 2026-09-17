//! The age policy that applies to the entries inside a cleanup unit.
//!
//! A cache namespace is written to while it is being cleaned: one file touched
//! this morning sits among gigabytes that have not changed in weeks. A verdict
//! about the whole tree cannot express that — it either discards everything or
//! nothing — so the policy is evaluated per entry, and the unit reports how much
//! of itself satisfies it.
//!
//! Two rules keep that from becoming "delete old files anywhere":
//!
//! * an entry's *own* age decides it, never its neighbours', and the same
//!   evaluation runs at scan time and immediately before the deletion, so the
//!   estimate a user saw and the bytes that go are produced by one function;
//! * an entry that is structured state is never stale-deletable, however old it
//!   is. A database, a write-ahead log, a lock, a credential, a configuration
//!   file, a bundle, or an executable is not cache warmth that went cold.

use std::time::{Duration, SystemTime};

use zenith_core::domain::cleanup::{classify_structured_state, EntryKind, PathFacts};

/// The threshold, as a duration, with the evaluation both callers share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleEntryPolicy {
    after: Duration,
}

impl StaleEntryPolicy {
    pub fn from_days(days: u32) -> Self {
        Self {
            after: Duration::from_secs(u64::from(days) * 86_400),
        }
    }

    pub fn after(&self) -> Duration {
        self.after
    }

    /// Whether one entry may be removed by this policy.
    ///
    /// The caller supplies what it observed about the entry: its name, what kind
    /// of entry it is, whether the platform marked it executable, and when it
    /// was last written. `None` for a missing timestamp fails closed — an entry
    /// whose age cannot be read is not an old entry.
    pub fn allows(
        &self,
        name: &str,
        entry_kind: EntryKind,
        executable: bool,
        modified: Option<SystemTime>,
        now: SystemTime,
    ) -> bool {
        if classify_structured_state(PathFacts::new(name, entry_kind).executable(executable))
            .is_some()
        {
            return false;
        }
        let Some(modified) = modified else {
            return false;
        };
        now.duration_since(modified)
            .map(|age| age >= self.after)
            .unwrap_or(false)
    }
}

/// Whether the platform marked a path executable, as the classifier needs.
pub fn is_executable(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

/// The entry kind of observed metadata, in the classifier's terms.
pub fn entry_kind(metadata: &std::fs::Metadata) -> EntryKind {
    if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000)
    }

    fn ago(days: u64) -> Option<SystemTime> {
        now().checked_sub(Duration::from_secs(days * 86_400))
    }

    #[test]
    fn an_entry_is_stale_only_when_its_own_age_satisfies_the_policy() {
        let policy = StaleEntryPolicy::from_days(7);
        assert!(policy.allows("blob", EntryKind::File, false, ago(8), now()));
        assert!(!policy.allows("blob", EntryKind::File, false, ago(6), now()));
        // The threshold is inclusive, as the tree-level verdict is: exactly
        // seven days old is old enough, one minute less is not.
        assert!(policy.allows("blob", EntryKind::File, false, ago(7), now()));
        assert!(!policy.allows(
            "blob",
            EntryKind::File,
            false,
            now().checked_sub(Duration::from_secs(7 * 86_400 - 60)),
            now()
        ));
        // A missing or future timestamp is not an old entry.
        assert!(!policy.allows("blob", EntryKind::File, false, None, now()));
        assert!(!policy.allows(
            "blob",
            EntryKind::File,
            false,
            now().checked_add(Duration::from_secs(60)),
            now()
        ));
    }

    #[test]
    fn structured_state_is_never_stale_deletable_however_old_it_is() {
        let policy = StaleEntryPolicy::from_days(1);
        for (name, kind, executable) in [
            ("Cache.db", EntryKind::File, false),
            ("cache.sqlite-wal", EntryKind::File, false),
            ("CURRENT", EntryKind::File, false),
            ("app.lock", EntryKind::File, false),
            ("auth.json", EntryKind::File, false),
            ("settings.json", EntryKind::File, false),
            ("Tool.app", EntryKind::Directory, false),
            ("helper", EntryKind::File, true),
        ] {
            assert!(
                !policy.allows(name, kind, executable, ago(365), now()),
                "{name} is structured state"
            );
        }
        // An ordinary cache blob is not protected by that rule.
        assert!(policy.allows("data_0", EntryKind::File, false, ago(365), now()));
    }
}
