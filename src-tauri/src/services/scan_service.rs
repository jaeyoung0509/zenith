use std::sync::Arc;

use crate::models::{CancellationProbe, ScanProgressSink, ScanRequest, ScanResult};
use crate::platform::PlatformEnvironment;
use crate::scanner::ScanEngine;
use crate::signatures::SignatureRegistry;

/// Application service responsible for running framework-independent system cleanup scans.
///
/// Encapsulates the signature catalog and environment description. Receives neutral
/// progress sinks and cancellation probes rather than desktop UI framework channels.
pub struct ScanService {
    registry: Arc<SignatureRegistry>,
    environment: Arc<PlatformEnvironment>,
}

impl ScanService {
    pub fn new(registry: Arc<SignatureRegistry>, environment: Arc<PlatformEnvironment>) -> Self {
        Self {
            registry,
            environment,
        }
    }

    pub fn registry(&self) -> &Arc<SignatureRegistry> {
        &self.registry
    }

    pub fn environment(&self) -> &Arc<PlatformEnvironment> {
        &self.environment
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
            request.categories.as_deref(),
            &request.excluded_signatures,
            request.intensive_cleanup,
            &self.environment,
            cancellation,
            move |event| progress.emit(event),
        )
    }
}
