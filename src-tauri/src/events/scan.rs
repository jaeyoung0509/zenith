//! Cleanup progress adapted onto a Tauri `Channel`.

use tauri::ipc::Channel;

use crate::models::{ScanEvent, ScanProgressSink};

/// Forwards scan progress to the channel the calling WebView owns.
pub struct TauriScanProgress {
    channel: Channel<ScanEvent>,
}

impl TauriScanProgress {
    pub fn new(channel: Channel<ScanEvent>) -> Self {
        Self { channel }
    }
}

impl ScanProgressSink for TauriScanProgress {
    fn emit(&self, event: ScanEvent) {
        // A closed channel means the WebView went away; the scan itself is
        // unaffected, and the result the command returns still carries the
        // outcome.
        let _ = self.channel.send(event);
    }
}
