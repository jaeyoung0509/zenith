//! The state a Tauri command receives.
//!
//! This type declares what an IPC adapter may reach: the platform facts the
//! composition root observed, the settings authority, and four bounded
//! services. It deliberately exposes no scan, inventory, plan store, cache, or
//! workflow mutex — those belong to the service that protects their invariant,
//! and a handler that reached one directly would be making a domain decision
//! the service exists to own.
//!
//! Construction lives in [`crate::composition`]; this module only declares the
//! shape.

use std::sync::{Arc, Mutex};

use crate::models::DashboardRoute;
use crate::services::{
    AiService, CleanupService, SettingsAuthority, StorageService, SystemService,
};
use crate::signatures::SignatureRegistry;

pub struct DesktopState {
    /// Platform facts the backend may depend on. Tests inject a simulated
    /// environment here instead of reading the host's.
    pub environment: Arc<zenith_platform::PlatformEnvironment>,
    /// The cleanup catalog this process loaded, for the startup refusal below
    /// and for callers that need the same catalog the scan will use.
    pub registry: Arc<SignatureRegistry>,
    /// Why the embedded signature catalog never loaded, when it did not.
    ///
    /// A catalog that failed is empty, and an empty catalog scans clean: the
    /// scan refuses while this is set instead of reporting a healthy machine.
    registry_load_error: Option<String>,
    /// The single in-memory copy of user preferences.
    pub settings: Arc<SettingsAuthority>,
    pub cleanup: Arc<CleanupService>,
    pub storage: Arc<StorageService>,
    pub ai: Arc<AiService>,
    pub system: Arc<SystemService>,
    /// The destination the dashboard must open on, when the surface that asked
    /// for the window named one.
    ///
    /// This is shell state rather than a use case: nothing in the application
    /// layer reads it. It exists because the destination is *pulled* by the
    /// window on mount instead of pushed as an event, so a slow webview load
    /// cannot lose the request it was supposed to render. Only the main window
    /// holds the grant that consumes it.
    pending_navigation: Mutex<Option<DashboardRoute>>,
}

impl DesktopState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        environment: Arc<zenith_platform::PlatformEnvironment>,
        registry: Arc<SignatureRegistry>,
        registry_load_error: Option<String>,
        settings: Arc<SettingsAuthority>,
        cleanup: Arc<CleanupService>,
        storage: Arc<StorageService>,
        ai: Arc<AiService>,
        system: Arc<SystemService>,
    ) -> Self {
        Self {
            environment,
            registry,
            registry_load_error,
            settings,
            cleanup,
            storage,
            ai,
            system,
            pending_navigation: Mutex::new(None),
        }
    }

    /// Records the destination the next activation of the dashboard must open
    /// on, replacing any destination that was never consumed.
    pub fn set_pending_navigation(&self, route: DashboardRoute) {
        let mut pending = self
            .pending_navigation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *pending = Some(route);
    }

    /// Consumes the pending destination, so a window that mounts twice cannot
    /// be sent to the same place twice.
    ///
    /// The value is disposable, so a lock poisoned by a panicking caller is
    /// recovered rather than propagated: the destination is still the one that
    /// was stored, and refusing to answer would strand the window on whatever
    /// page it already shows.
    pub fn take_pending_navigation(&self) -> Option<DashboardRoute> {
        let mut pending = self
            .pending_navigation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.take()
    }

    /// The reason a scan cannot run, when the signature catalog never loaded.
    ///
    /// An empty catalog scans nothing and reports `Fresh`, which is exactly
    /// what a clean machine reports; refusing the scan is what keeps a startup
    /// failure from being presented as one.
    pub fn catalog_failure(&self) -> Option<String> {
        self.registry_load_error.as_deref().map(|error| {
            format!("{error}. Scan and cleanup are unavailable until the signature catalog loads.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::adapter::ContainerHost;

    fn state() -> DesktopState {
        crate::composition::desktop_state(
            Arc::new(zenith_platform::PlatformEnvironment::native()),
            ContainerHost::unstated(),
        )
    }

    #[test]
    fn a_stored_destination_is_consumed_exactly_once() {
        let state = state();

        assert_eq!(
            state.take_pending_navigation(),
            None,
            "a shell that was never asked to open anywhere has no destination"
        );

        state.set_pending_navigation(DashboardRoute::Settings);
        assert_eq!(
            state.take_pending_navigation(),
            Some(DashboardRoute::Settings)
        );
        assert_eq!(
            state.take_pending_navigation(),
            None,
            "the destination is one-shot, so a second mount cannot be sent to it again"
        );
    }

    #[test]
    fn the_newest_destination_replaces_one_that_was_never_consumed() {
        let state = state();

        state.set_pending_navigation(DashboardRoute::Memory);
        state.set_pending_navigation(DashboardRoute::Settings);

        assert_eq!(
            state.take_pending_navigation(),
            Some(DashboardRoute::Settings),
            "the window must open on the destination the last request named"
        );
    }

    #[test]
    fn a_poisoned_navigation_lock_still_answers() {
        let state = state();

        // Poison the lock the way a panicking holder would: the state is
        // disposable, so the destination must survive it.
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = state.pending_navigation.lock().expect("the lock is clean");
            panic!("the holder died while writing the destination");
        }));
        assert!(poisoned.is_err(), "the holder panicked");

        state.set_pending_navigation(DashboardRoute::Settings);
        assert_eq!(
            state.take_pending_navigation(),
            Some(DashboardRoute::Settings)
        );
    }
}
