use std::sync::Arc;

use crate::cleaner::LifecycleProviderRegistry;
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
    environment: Arc<PlatformEnvironment>,
}

impl ScanService {
    pub fn new(
        registry: Arc<SignatureRegistry>,
        lifecycle_providers: Arc<LifecycleProviderRegistry>,
        environment: Arc<PlatformEnvironment>,
    ) -> Self {
        Self {
            registry,
            lifecycle_providers,
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
        ScanEngine::scan(
            &self.registry,
            &self.lifecycle_providers,
            request.categories.as_deref(),
            &request.excluded_signatures,
            request.intensive_cleanup,
            &self.environment,
            cancellation,
            move |event| progress.emit(event),
        )
    }
}
