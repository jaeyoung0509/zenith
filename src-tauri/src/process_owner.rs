//! Platform-aware process owner identity.
//!
//! Unix processes are owned by an effective UID. Windows processes are owned
//! by the SID captured from the process token. Windows ownership is verified
//! by comparing the candidate SID with the current process SID; it is never
//! represented as a fake Unix UID.

use sysinfo::Uid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOwner {
    Unix(u32),
    Windows(String),
}

impl ProcessOwner {
    /// Returns the owner of the current Zenith process.
    ///
    /// On Windows this performs a best-effort sysinfo lookup of the current
    /// process SID. When the SID is unavailable an empty sentinel is returned
    /// so every ownership comparison safely fails closed.
    pub fn current() -> Self {
        #[cfg(unix)]
        {
            // SAFETY: geteuid has no failure modes.
            unsafe { Self::Unix(libc::geteuid()) }
        }
        #[cfg(not(unix))]
        {
            Self::Windows(current_process_sid().unwrap_or_default())
        }
    }

    /// Verifies a candidate process owner against the current process owner.
    ///
    /// On Unix the candidate UID is parsed and returned. On Windows the
    /// candidate SID must exactly equal the current process SID; otherwise
    /// `None` is returned. This reuses the proven sysinfo SID comparison
    /// pattern already used by agent activity.
    pub fn verified(candidate: Option<&Uid>, own: Option<&Uid>) -> Option<Self> {
        #[cfg(unix)]
        {
            let _ = own;
            candidate?.to_string().parse::<u32>().ok().map(Self::Unix)
        }
        #[cfg(not(unix))]
        {
            match (candidate, own) {
                (Some(candidate), Some(own)) if candidate == own => {
                    Some(Self::Windows(candidate.to_string()))
                }
                _ => None,
            }
        }
    }

    /// Returns the Unix UID when this owner is a Unix identity.
    pub fn as_unix_uid(&self) -> Option<u32> {
        match self {
            Self::Unix(uid) => Some(*uid),
            Self::Windows(_) => None,
        }
    }

    /// Returns true when this owner is the privileged system owner.
    pub fn is_privileged(&self) -> bool {
        match self {
            Self::Unix(uid) => *uid == 0,
            // An empty Windows sentinel means the SID was unavailable and must
            // never be treated as an authorized owner.
            Self::Windows(sid) => sid.is_empty(),
        }
    }
}

#[cfg(not(unix))]
fn current_process_sid() -> Option<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new();
    let own_pid = Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[own_pid]),
        true,
        ProcessRefreshKind::everything(),
    );
    let process = sys.process(own_pid)?;
    let uid = process.effective_user_id().or_else(|| process.user_id())?;
    Some(uid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn unix_owner_verification_maps_the_candidate_uid_and_fails_closed_without_one() {
        let uid: Uid = "501".parse().expect("uid parses");
        let own: Uid = "999".parse().expect("uid parses");

        // The Unix identity is the candidate's own euid; whether that euid is
        // authorized is decided by the caller comparing whole owners.
        assert_eq!(
            ProcessOwner::verified(Some(&uid), Some(&own)),
            Some(ProcessOwner::Unix(501))
        );
        assert_eq!(
            ProcessOwner::verified(Some(&uid), None),
            Some(ProcessOwner::Unix(501))
        );
        assert_eq!(ProcessOwner::verified(None, Some(&own)), None);
        assert_eq!(ProcessOwner::verified(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn unix_current_owner_matches_euid() {
        let expected = unsafe { libc::geteuid() };
        assert_eq!(ProcessOwner::current(), ProcessOwner::Unix(expected));
        assert_eq!(
            ProcessOwner::current().as_unix_uid(),
            Some(expected),
            "a Unix owner must expose its UID"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_owner_requires_exact_sid_match() {
        let own: Uid = "S-1-5-21-100".parse().expect("sid parses");
        let other: Uid = "S-1-5-21-200".parse().expect("sid parses");
        assert_eq!(
            ProcessOwner::verified(Some(&own), Some(&own)),
            Some(ProcessOwner::Windows("S-1-5-21-100".to_string()))
        );
        assert_eq!(ProcessOwner::verified(Some(&other), Some(&own)), None);
        assert_eq!(ProcessOwner::verified(None, Some(&own)), None);
        assert_eq!(ProcessOwner::verified(Some(&own), None), None);
    }

    #[test]
    fn an_unavailable_windows_sid_is_never_verified_and_is_privileged() {
        // `current()` returns this sentinel when the process SID cannot be
        // read. Callers must treat it as "no verified identity": it compares
        // equal to no real SID and is refused as a privileged owner rather
        // than being accepted as the current user.
        let unavailable = ProcessOwner::Windows(String::new());
        assert!(unavailable.is_privileged());
        assert_ne!(unavailable, ProcessOwner::Windows("S-1-5-21-100".into()));
        assert_eq!(unavailable.as_unix_uid(), None);
        assert!(!ProcessOwner::Unix(501).is_privileged());
        assert!(ProcessOwner::Unix(0).is_privileged());
    }
}
