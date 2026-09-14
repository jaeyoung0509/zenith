//! Cleanup execution progress adapted onto a Tauri `Channel`.

use tauri::ipc::Channel;

use crate::models::{CleanEvent, CleanupProgressSink};

/// Forwards cleanup progress to the channel the calling WebView owns.
pub struct TauriCleanupProgress {
    channel: Channel<CleanEvent>,
}

impl TauriCleanupProgress {
    pub fn new(channel: Channel<CleanEvent>) -> Self {
        Self { channel }
    }
}

impl CleanupProgressSink for TauriCleanupProgress {
    fn emit(&self, event: CleanEvent) {
        let _ = self.channel.send(event);
    }
}
