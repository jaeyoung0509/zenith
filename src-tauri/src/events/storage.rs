//! Reviewed-storage progress adapted onto a Tauri `Channel`.

use tauri::ipc::Channel;

use crate::models::{DeveloperArtifactScanEvent, LargeFileScanEvent};
use crate::services::progress::{DeveloperArtifactScanSink, LargeFileScanSink};

/// Forwards large-file scan progress to the channel the WebView owns.
pub struct TauriLargeFileScanProgress {
    channel: Channel<LargeFileScanEvent>,
}

impl TauriLargeFileScanProgress {
    pub fn new(channel: Channel<LargeFileScanEvent>) -> Self {
        Self { channel }
    }
}

impl LargeFileScanSink for TauriLargeFileScanProgress {
    fn emit(&self, event: LargeFileScanEvent) {
        let _ = self.channel.send(event);
    }
}

/// Forwards developer-artifact scan progress to the channel the WebView owns.
pub struct TauriDeveloperArtifactProgress {
    channel: Channel<DeveloperArtifactScanEvent>,
}

impl TauriDeveloperArtifactProgress {
    pub fn new(channel: Channel<DeveloperArtifactScanEvent>) -> Self {
        Self { channel }
    }
}

impl DeveloperArtifactScanSink for TauriDeveloperArtifactProgress {
    fn emit(&self, event: DeveloperArtifactScanEvent) {
        let _ = self.channel.send(event);
    }
}
