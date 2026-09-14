//! Progress sinks the application services emit into.
//!
//! Cleanup and scan events are domain events, so their sinks are defined in
//! `zenith_core` next to the events themselves. The reviewed-storage and
//! provider-usage events are desktop DTOs, so their sinks live here.
//!
//! A service emits a domain event and stops there; only [`crate::events`]
//! knows the transport is a Tauri `Channel`.

use crate::models::{AiProviderUsage, DeveloperArtifactScanEvent, LargeFileScanEvent};

pub trait LargeFileScanSink: Send + Sync {
    fn emit(&self, event: LargeFileScanEvent);
}

pub trait DeveloperArtifactScanSink: Send + Sync {
    fn emit(&self, event: DeveloperArtifactScanEvent);
}

pub trait ProviderUsageSink: Send + Sync {
    fn emit(&self, usage: AiProviderUsage);
}

impl<F> LargeFileScanSink for F
where
    F: Fn(LargeFileScanEvent) + Send + Sync,
{
    fn emit(&self, event: LargeFileScanEvent) {
        self(event);
    }
}

impl<F> DeveloperArtifactScanSink for F
where
    F: Fn(DeveloperArtifactScanEvent) + Send + Sync,
{
    fn emit(&self, event: DeveloperArtifactScanEvent) {
        self(event);
    }
}

impl<F> ProviderUsageSink for F
where
    F: Fn(AiProviderUsage) + Send + Sync,
{
    fn emit(&self, usage: AiProviderUsage) {
        self(usage);
    }
}
