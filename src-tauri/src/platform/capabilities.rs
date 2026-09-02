//! Native platform capability provider selected at the composition boundary.

use super::PlatformCapabilitiesProvider;
use crate::models::PlatformCapabilities;

#[derive(Debug, Clone, Copy, Default)]
pub struct NativePlatformCapabilities;

impl NativePlatformCapabilities {
    pub fn new() -> Self {
        Self
    }
}

impl PlatformCapabilitiesProvider for NativePlatformCapabilities {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::current()
    }
}
