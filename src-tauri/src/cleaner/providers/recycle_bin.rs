//! The Windows Recycle Bin, as a lifecycle-aware cleanup provider.
//!
//! The Recycle Bin is the first — and deliberately the only — provider this
//! build implements. It earns that place because every part of the contract is
//! real for it: its contents belong to the shell rather than to the profile, a
//! generic delete against `$Recycle.Bin` would corrupt the shell's own
//! bookkeeping instead of reclaiming the space, the amount it holds is
//! measurable through a supported interface, and the operation is irreversible
//! enough that it must never be pre-selected.
//!
//! The remaining candidates the catalog reports (the Windows Update payload
//! store, the delivery-optimization cache, WSL and Docker virtual disks)
//! stay advisory until each has its own adapter: they need a service stopped,
//! a tool's own compaction lifecycle, or per-product awareness that no shell
//! call provides. Discovery without deletion permission is the honest state for
//! them, and nothing here is a fallback for one of them.

use super::LifecycleProvider;
use std::sync::Arc;
use zenith_core::domain::cleanup::{ProviderOutcome, ProviderProbe, ProviderStatus};
use zenith_platform::{
    NativeRecycleBinBackend, PlatformEnvironment, RecycleBinBackend, RecycleBinError,
};

/// The stable id the catalog names for this action.
pub const PROVIDER_ID: &str = "windows.recycle_bin";

/// The location the reviewed action covers.
///
/// It is a pseudo path because the Recycle Bin has no single host path: the
/// shell owns one bin per volume, indexed per account, and the supported
/// interface addresses all of them. Nothing resolves this string as a path.
pub const PROVIDER_LOCATION: &str = "recycle-bin://all-volumes";

/// What running this action does, in the words the user is shown.
pub const CONSEQUENCE: &str = "Everything the Recycle Bin holds is permanently deleted. Windows does not keep another copy, and Zenith cannot undo this.";

pub struct WindowsRecycleBinProvider {
    backend: Arc<dyn RecycleBinBackend>,
}

impl WindowsRecycleBinProvider {
    /// The provider a user's installation ships.
    pub fn native() -> Self {
        Self {
            backend: Arc::new(NativeRecycleBinBackend),
        }
    }

    /// The provider over a stated backend, so its contract can be exercised on
    /// any host without touching a real Recycle Bin.
    pub fn with_backend(backend: Arc<dyn RecycleBinBackend>) -> Self {
        Self { backend }
    }
}

impl LifecycleProvider for WindowsRecycleBinProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn platforms(&self) -> &'static [zenith_core::domain::platform::PlatformKind] {
        &[zenith_core::domain::platform::PlatformKind::Windows]
    }

    fn consequence(&self) -> &'static str {
        CONSEQUENCE
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    fn location(&self) -> &'static str {
        PROVIDER_LOCATION
    }

    fn probe(&self, _environment: &PlatformEnvironment) -> ProviderProbe {
        // The bin's scope is the shell's, not the profile's: this provider
        // reads no environment fact, and a simulated environment cannot make a
        // host without the shell interface answer as if it had one.
        match self.backend.observe() {
            Ok(observation) => ProviderProbe::ready(observation.bytes, observation.item_count),
            Err(RecycleBinError::Unsupported) => ProviderProbe::refused(
                ProviderStatus::Unsupported,
                RecycleBinError::Unsupported.describe(),
            ),
            Err(RecycleBinError::Failed(message)) => {
                ProviderProbe::refused(ProviderStatus::Blocked, message)
            }
        }
    }

    fn execute(&self, _environment: &PlatformEnvironment) -> ProviderOutcome {
        // Prerequisites are re-derived here, at execution time, from the store
        // itself: the plan's measurement is what the scan saw and is never
        // trusted as the current state.
        let before = match self.backend.observe() {
            Ok(observation) => observation,
            Err(RecycleBinError::Unsupported) => {
                return ProviderOutcome::refused(
                    ProviderStatus::Unsupported,
                    RecycleBinError::Unsupported.describe(),
                )
            }
            Err(RecycleBinError::Failed(message)) => {
                return ProviderOutcome::refused(ProviderStatus::Failed, message)
            }
        };

        if before.bytes == 0 {
            // The postcondition already holds. Reporting a skip here would be
            // wrong: the reviewed plan asked for an empty Recycle Bin, and an
            // empty Recycle Bin is what it has.
            return ProviderOutcome::cleaned(0, Some(0))
                .with_detail("The Recycle Bin was already empty; nothing was removed.");
        }

        let emptied = self.backend.empty();

        // Verification decides what this run is: the shell reports an
        // already-empty bin as an error in some versions, so the return code
        // alone would turn a successful no-op into a failure report.
        match self.backend.observe() {
            Ok(after) if after.bytes == 0 => {
                let outcome = ProviderOutcome::cleaned(before.bytes, Some(0));
                match emptied {
                    Ok(()) => outcome,
                    Err(error) => outcome.with_detail(format!(
                        "{} The Recycle Bin was verified empty afterwards.",
                        error.describe()
                    )),
                }
            }
            Ok(after) => {
                let reclaimed = before.bytes.saturating_sub(after.bytes);
                match emptied {
                    Ok(()) => ProviderOutcome::partially_cleaned(
                        reclaimed,
                        Some(after.bytes),
                        format!(
                            "{} bytes remain in the Recycle Bin after the action ran",
                            after.bytes
                        ),
                    ),
                    Err(error) if reclaimed > 0 => ProviderOutcome::partially_cleaned(
                        reclaimed,
                        Some(after.bytes),
                        format!(
                            "{} {} bytes remain in the Recycle Bin after partial progress.",
                            error.describe(),
                            after.bytes
                        ),
                    ),
                    Err(error) => ProviderOutcome::refused(
                        ProviderStatus::Failed,
                        format!(
                            "{} {} bytes remain in the Recycle Bin.",
                            error.describe(),
                            after.bytes
                        ),
                    ),
                }
            }
            Err(error) => match emptied {
                // The action was accepted but its result could not be read:
                // the run is partial, and no reclaimed amount is claimed for
                // it, because nothing measured one.
                Ok(()) => ProviderOutcome::partially_cleaned(
                    0,
                    None,
                    format!(
                        "The Recycle Bin was emptied but the remaining size could not be read: {}",
                        error.describe()
                    ),
                ),
                Err(error) => ProviderOutcome::refused(
                    ProviderStatus::Failed,
                    format!(
                        "{} The Recycle Bin could not be verified afterwards.",
                        error.describe()
                    ),
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowsRecycleBinProvider, CONSEQUENCE};
    use crate::cleaner::providers::LifecycleProvider;
    use std::sync::Arc;
    use zenith_core::domain::cleanup::ProviderStatus;
    use zenith_platform::path_algebra::PathFlavor;
    use zenith_platform::{
        MockRecycleBinBackend, PlatformEnvironment, RecycleBinBackend, RecycleBinError,
    };

    fn environment() -> PlatformEnvironment {
        PlatformEnvironment::simulated(PathFlavor::current())
    }

    fn provider_over(backend: Arc<dyn RecycleBinBackend>) -> WindowsRecycleBinProvider {
        WindowsRecycleBinProvider::with_backend(backend)
    }

    /// The probe is the provider's own measurement, and a store it cannot read
    /// is refused rather than reported as empty. It is also a dry run: reading
    /// the store is the whole of it.
    #[test]
    fn the_probe_reports_the_stores_own_measurement() {
        let holding = Arc::new(MockRecycleBinBackend::holding(5_000, 7));
        let provider = provider_over(holding.clone());
        let probe = provider.probe(&environment());
        assert!(probe.status.is_ready());
        assert_eq!(probe.estimated_bytes, 5_000);
        assert_eq!(probe.item_count, 7);
        assert_eq!(
            holding.empty_calls(),
            0,
            "probing a store must never mutate it"
        );

        let provider = provider_over(Arc::new(MockRecycleBinBackend::unsupported()));
        let probe = provider.probe(&environment());
        assert_eq!(probe.status, ProviderStatus::Unsupported);
        assert_eq!(probe.estimated_bytes, 0);

        let provider = provider_over(Arc::new(MockRecycleBinBackend::failing_observe(
            RecycleBinError::Failed("the shell refused the query".to_string()),
        )));
        let probe = provider.probe(&environment());
        assert_eq!(probe.status, ProviderStatus::Blocked);
        assert_eq!(
            probe.detail.as_deref(),
            Some("the shell refused the query"),
            "the refusal keeps the interface's own words"
        );
    }

    /// The declared facts are what the catalog and the interface rely on: a
    /// stable id, the platform, the consequence, and the confirmation.
    #[test]
    fn the_action_declares_its_platform_consequence_and_confirmation() {
        let provider = provider_over(Arc::new(MockRecycleBinBackend::holding(1, 1)));
        assert_eq!(provider.id(), "windows.recycle_bin");
        assert_eq!(
            provider.platforms(),
            &[zenith_core::domain::platform::PlatformKind::Windows]
        );
        assert!(provider.requires_confirmation());
        assert_eq!(provider.consequence(), CONSEQUENCE);
        assert!(provider.consequence().contains("permanently deleted"));
        assert_eq!(provider.location(), "recycle-bin://all-volumes");
    }

    /// A run that empties the bin states what verification measured, and a bin
    /// that was empty before the action claims nothing.
    #[test]
    fn a_clean_run_reports_what_verification_observed() {
        let backend = Arc::new(MockRecycleBinBackend::holding(8_192, 2));
        let provider = provider_over(backend.clone());

        let outcome = provider.execute(&environment());

        assert_eq!(outcome.status, ProviderStatus::Cleaned);
        assert_eq!(outcome.reclaimed_bytes, 8_192);
        assert_eq!(outcome.remaining_bytes, Some(0));
        assert_eq!(backend.empty_calls(), 1);
        assert_eq!(
            backend.observe_calls(),
            2,
            "the run re-read the store before acting and verified afterwards"
        );

        let empty = Arc::new(MockRecycleBinBackend::holding(0, 0));
        let provider = provider_over(empty.clone());
        let outcome = provider.execute(&environment());
        assert_eq!(outcome.status, ProviderStatus::Cleaned);
        assert_eq!(outcome.reclaimed_bytes, 0);
        assert_eq!(
            empty.empty_calls(),
            0,
            "an empty bin is not asked to be emptied again"
        );
        assert!(outcome.detail.is_some(), "the no-op states itself");
    }

    /// The interface's return code is not the measurement: a shell that reports
    /// an error while the bin is in fact empty is a clean run, and one that
    /// reports success while bytes remain is a partial run.
    #[test]
    fn verification_decides_the_outcome_not_the_return_code() {
        let reported_error = Arc::new(MockRecycleBinBackend::holding(4_096, 2).with_empty_error(
            RecycleBinError::Failed("the Recycle Bin was already empty".to_string()),
            0,
        ));
        let provider = provider_over(reported_error);
        let outcome = provider.execute(&environment());
        assert_eq!(outcome.status, ProviderStatus::Cleaned);
        assert!(outcome
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("verified empty")));

        let left_something =
            Arc::new(MockRecycleBinBackend::holding(4_096, 2).with_empty_remaining(1_024));
        let provider = provider_over(left_something);
        let outcome = provider.execute(&environment());
        assert_eq!(outcome.status, ProviderStatus::PartiallyCleaned);
        assert_eq!(outcome.reclaimed_bytes, 4_096 - 1_024);
        assert_eq!(outcome.remaining_bytes, Some(1_024));
        assert!(outcome
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("1024")));

        let errored_after_partial = Arc::new(
            MockRecycleBinBackend::holding(4_096, 2).with_empty_error(
                RecycleBinError::Failed("shell returned an error".to_string()),
                1_024,
            ),
        );
        let provider = provider_over(errored_after_partial);
        let outcome = provider.execute(&environment());
        assert_eq!(outcome.status, ProviderStatus::PartiallyCleaned);
        assert_eq!(outcome.reclaimed_bytes, 4_096 - 1_024);
        assert_eq!(outcome.remaining_bytes, Some(1_024));
        assert!(outcome
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("shell returned an error")));
    }

    /// A refused action claims nothing: the bytes it could not remove are not
    /// reported as reclaimed, and the refusal carries the interface's words.
    #[test]
    fn a_refused_action_reclaims_nothing_and_says_why() {
        let refused = Arc::new(
            MockRecycleBinBackend::holding(2_048, 1)
                .with_empty_error(RecycleBinError::Failed("access denied".to_string()), 2_048),
        );
        let provider = provider_over(refused);
        let outcome = provider.execute(&environment());
        assert_eq!(outcome.status, ProviderStatus::Failed);
        assert_eq!(outcome.reclaimed_bytes, 0);
        assert!(outcome
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("access denied")));

        let unsupported = provider_over(Arc::new(MockRecycleBinBackend::unsupported()));
        let outcome = unsupported.execute(&environment());
        assert_eq!(outcome.status, ProviderStatus::Unsupported);
        assert_eq!(outcome.reclaimed_bytes, 0);
    }
}
