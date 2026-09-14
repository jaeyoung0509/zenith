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

use std::sync::Arc;

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
        }
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
