use std::sync::Arc;

use crate::cache_providers::{CacheProviderRegistry, CacheProviderScanner};
use crate::cleaner::{LifecycleProviderRegistry, OwnerProviderRegistry};
use crate::models::{CancellationProbe, ScanProgressSink, ScanRequest, ScanResult};
use crate::scanner::ScanEngine;
use crate::signatures::SignatureRegistry;
use zenith_platform::PlatformEnvironment;

/// Application service responsible for running framework-independent system cleanup scans.
///
/// Encapsulates the signature catalog and environment description. Receives neutral
/// progress sinks and cancellation probes rather than desktop UI framework channels.
pub struct ScanService {
    registry: Arc<SignatureRegistry>,
    lifecycle_providers: Arc<LifecycleProviderRegistry>,
    owner_providers: Arc<OwnerProviderRegistry>,
    cache_providers: Arc<dyn CacheProviderScanner>,
    environment: Arc<PlatformEnvironment>,
}

impl ScanService {
    pub fn new(
        registry: Arc<SignatureRegistry>,
        lifecycle_providers: Arc<LifecycleProviderRegistry>,
        owner_providers: Arc<OwnerProviderRegistry>,
        environment: Arc<PlatformEnvironment>,
    ) -> Self {
        Self::new_with_cache_providers(
            registry,
            lifecycle_providers,
            owner_providers,
            Arc::new(CacheProviderRegistry),
            environment,
        )
    }

    pub(crate) fn new_with_cache_providers(
        registry: Arc<SignatureRegistry>,
        lifecycle_providers: Arc<LifecycleProviderRegistry>,
        owner_providers: Arc<OwnerProviderRegistry>,
        cache_providers: Arc<dyn CacheProviderScanner>,
        environment: Arc<PlatformEnvironment>,
    ) -> Self {
        Self {
            registry,
            lifecycle_providers,
            owner_providers,
            cache_providers,
            environment,
        }
    }

    /// Executes a scan synchronously using the provided request, progress sink, and cancellation probe.
    pub fn scan(
        &self,
        request: &ScanRequest,
        progress: &dyn ScanProgressSink,
        cancellation: &dyn CancellationProbe,
    ) -> ScanResult {
        ScanEngine::scan_with_cache_providers(
            &self.registry,
            &self.lifecycle_providers,
            &self.owner_providers,
            request.categories.as_deref(),
            &request.excluded_signatures,
            request.intensive_cleanup,
            &self.environment,
            cancellation,
            self.cache_providers.as_ref(),
            move |event| progress.emit(event),
        )
    }

    pub fn merge_slices(&self, slices: &[ScanResult]) -> Option<ScanResult> {
        ScanEngine::merge_slices(&self.registry, &self.environment, slices)
    }
}
