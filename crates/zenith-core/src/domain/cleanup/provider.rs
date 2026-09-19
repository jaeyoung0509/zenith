//! What a lifecycle-aware provider observed, and how its action ended.
//!
//! Some reclaimable storage is not a directory: it is a store an operating
//! system, a shell, or an application owns, and removing it safely means asking
//! its owner to do it. The provider contract exists so those units are
//! discovered and measured like everything else, while the mutation itself
//! stays with the component that understands the store.
//!
//! The vocabulary here is deliberately closed. A provider does not return an
//! error string that a caller has to interpret, and it never reports success
//! for something it did not verify: it states one of a fixed set of statuses,
//! and the two facts that decide what the interface may claim — how much the
//! run reclaimed, and what its own verification observed afterwards — travel
//! with that status.
//!
//! Two rules the shapes enforce:
//!
//! * a probe that is not [`ProviderStatus::Ready`] carries no byte estimate,
//!   because an estimate nothing measured is not a measurement;
//! * an outcome states what verification observed (`remaining_bytes`) rather
//!   than implying it, so an unverifiable run is representable — and is
//!   reported as partial — instead of being presented as a clean one.

/// How a provider's probe or action ended.
///
/// The states are the ones the interface has to be able to distinguish: not
/// supported here, not ready yet, could not be read, ran and finished, ran and
/// left something behind, ran and failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus {
    /// Prerequisites hold and the provider read its own state.
    Ready,
    /// The provider exists, but a prerequisite it states does not hold yet
    /// (a component that must be stopped, a session that must exist).
    PrerequisiteNotMet,
    /// The provider could not read or reach the store it owns.
    Blocked,
    /// This platform or build has no adapter that can perform the action.
    Unsupported,
    /// The action ran and part of what it covers remains.
    PartiallyCleaned,
    /// The action ran and its postcondition holds.
    Cleaned,
    /// The action ran and did not reach its postcondition.
    Failed,
}

impl ProviderStatus {
    /// Whether the provider may run now.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Whether the action completed without leaving anything behind.
    pub fn is_cleaned(&self) -> bool {
        matches!(self, Self::Cleaned)
    }

    /// Whether this status is an answer to a probe rather than to an action.
    pub fn is_probe_state(&self) -> bool {
        matches!(
            self,
            Self::Ready | Self::PrerequisiteNotMet | Self::Blocked | Self::Unsupported
        )
    }

    /// The phrase a message uses for this status.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::PrerequisiteNotMet => "a prerequisite is not met",
            Self::Blocked => "blocked",
            Self::Unsupported => "not supported on this platform",
            Self::PartiallyCleaned => "partially cleaned",
            Self::Cleaned => "cleaned",
            Self::Failed => "failed",
        }
    }
}

/// What a provider's probe observed about the store it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbe {
    pub status: ProviderStatus,
    /// What the provider estimates it could reclaim. Populated only when the
    /// probe is [`ProviderStatus::Ready`].
    pub estimated_bytes: u64,
    /// How many items the estimate covers, when the interface reports a count.
    pub item_count: u64,
    /// Whether `estimated_bytes` is a lower bound rather than a measurement.
    pub bytes_are_lower_bound: bool,
    /// The provider's own words about a status that is not ready.
    pub detail: Option<String>,
}

impl ProviderProbe {
    /// A probe that read the store and can run.
    pub fn ready(estimated_bytes: u64, item_count: u64) -> Self {
        Self {
            status: ProviderStatus::Ready,
            estimated_bytes,
            item_count,
            bytes_are_lower_bound: false,
            detail: None,
        }
    }

    /// A probe that cannot run, and why.
    ///
    /// The estimate is zero by construction: without reading the store there is
    /// nothing to report, and a number nothing measured must not reach a total.
    pub fn refused(status: ProviderStatus, detail: impl Into<String>) -> Self {
        debug_assert!(
            status.is_probe_state() && !status.is_ready(),
            "a refused probe states a probe status that is not `Ready`"
        );
        Self {
            status,
            estimated_bytes: 0,
            item_count: 0,
            bytes_are_lower_bound: false,
            detail: Some(detail.into()),
        }
    }

    /// Whether the probe found bytes a cleanup could reclaim.
    ///
    /// A ready probe with nothing to reclaim is a real answer — the store is
    /// empty — and it is not an item.
    pub fn has_reclaimable_bytes(&self) -> bool {
        self.status.is_ready() && self.estimated_bytes > 0
    }
}

/// The result of one provider action, verification included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOutcome {
    pub status: ProviderStatus,
    /// What the run removed, measured by re-reading the store afterwards.
    pub reclaimed_bytes: u64,
    /// What the provider observed remaining, when its verification could read
    /// the store. `None` means the run could not be verified, which is
    /// reported as partial rather than as clean.
    pub remaining_bytes: Option<u64>,
    pub detail: Option<String>,
}

impl ProviderOutcome {
    /// The action ran and verification observed the stated remainder.
    pub fn cleaned(reclaimed_bytes: u64, remaining_bytes: Option<u64>) -> Self {
        Self {
            status: ProviderStatus::Cleaned,
            reclaimed_bytes,
            remaining_bytes,
            detail: None,
        }
    }

    /// States the provider's own words about this outcome.
    ///
    /// A clean run uses it when the interface's return code and the verified
    /// state disagree — an already-empty bin some shells report as an error —
    /// so the record says what was observed instead of implying a silent
    /// success.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The action ran and something it covers remains, or could not be proven
    /// gone. The reclaimed amount measured so far is stated.
    pub fn partially_cleaned(
        reclaimed_bytes: u64,
        remaining_bytes: Option<u64>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            status: ProviderStatus::PartiallyCleaned,
            reclaimed_bytes,
            remaining_bytes,
            detail: Some(detail.into()),
        }
    }

    /// The action did not run, or ran and failed. Nothing was verified as
    /// removed, so the reclaimed amount is zero.
    pub fn refused(status: ProviderStatus, detail: impl Into<String>) -> Self {
        debug_assert!(
            !status.is_cleaned() && status != ProviderStatus::PartiallyCleaned,
            "a refused outcome states neither `Cleaned` nor `PartiallyCleaned`"
        );
        Self {
            status,
            reclaimed_bytes: 0,
            remaining_bytes: None,
            detail: Some(detail.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderOutcome, ProviderProbe, ProviderStatus};

    /// The status vocabulary is closed, and the two questions a caller asks of
    /// it — may this run, and did it finish — are answered by the status alone.
    #[test]
    fn the_status_vocabulary_separates_probe_answers_from_outcomes() {
        let probe_states = [
            ProviderStatus::Ready,
            ProviderStatus::PrerequisiteNotMet,
            ProviderStatus::Blocked,
            ProviderStatus::Unsupported,
        ];
        for state in probe_states {
            assert!(state.is_probe_state(), "{state:?} answers a probe");
            assert_eq!(state.is_ready(), state == ProviderStatus::Ready);
            assert!(!state.is_cleaned());
        }

        let outcome_states = [ProviderStatus::PartiallyCleaned, ProviderStatus::Cleaned];
        for state in outcome_states {
            assert!(!state.is_probe_state(), "{state:?} answers an action");
        }
        assert!(ProviderStatus::Cleaned.is_cleaned());
        assert!(!ProviderStatus::PartiallyCleaned.is_cleaned());
        assert!(!ProviderStatus::Failed.is_probe_state());
        assert!(ProviderStatus::Unsupported
            .display_name()
            .contains("not supported"));
    }

    /// A probe that could not read the store reports no bytes: an estimate
    /// nothing measured never enters a total.
    #[test]
    fn a_refused_probe_states_no_estimate() {
        let refused = ProviderProbe::refused(
            ProviderStatus::Blocked,
            "the shell interface refused the query",
        );
        assert!(!refused.status.is_ready());
        assert_eq!(refused.estimated_bytes, 0);
        assert!(!refused.has_reclaimable_bytes());
        assert_eq!(
            refused.detail.as_deref(),
            Some("the shell interface refused the query")
        );

        // A ready probe with nothing in the store is a real answer, and it is
        // still not an item to offer.
        let empty = ProviderProbe::ready(0, 0);
        assert!(empty.status.is_ready());
        assert!(!empty.has_reclaimable_bytes());

        let found = ProviderProbe::ready(4_096, 3);
        assert!(found.has_reclaimable_bytes());
        assert_eq!(found.item_count, 3);
        assert!(found.detail.is_none());
    }

    /// Verification is part of the outcome: a run whose remainder could not be
    /// read is partial, never clean, and a refused action claims no bytes.
    #[test]
    fn an_outcome_states_what_verification_observed() {
        let cleaned = ProviderOutcome::cleaned(8_192, Some(0));
        assert_eq!(cleaned.status, ProviderStatus::Cleaned);
        assert_eq!(cleaned.reclaimed_bytes, 8_192);
        assert_eq!(cleaned.remaining_bytes, Some(0));

        let unverifiable =
            ProviderOutcome::partially_cleaned(8_192, None, "the remaining size could not be read");
        assert_eq!(unverifiable.status, ProviderStatus::PartiallyCleaned);
        assert_eq!(unverifiable.remaining_bytes, None);
        assert!(unverifiable.detail.is_some());

        let refused = ProviderOutcome::refused(
            ProviderStatus::PrerequisiteNotMet,
            "the owning process is running",
        );
        assert_eq!(refused.reclaimed_bytes, 0);
        assert_eq!(refused.remaining_bytes, None);
        assert!(!refused.status.is_cleaned());
    }
}
