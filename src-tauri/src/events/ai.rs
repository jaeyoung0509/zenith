//! AI usage progress adapted onto a Tauri `Channel`.

use tauri::ipc::Channel;

use crate::models::AiProviderUsage;
use crate::services::progress::ProviderUsageSink;

/// Forwards incremental provider observations to the channel the WebView owns.
pub struct TauriProviderUsageProgress {
    channel: Channel<AiProviderUsage>,
}

impl TauriProviderUsageProgress {
    pub fn new(channel: Channel<AiProviderUsage>) -> Self {
        Self { channel }
    }
}

impl ProviderUsageSink for TauriProviderUsageProgress {
    fn emit(&self, usage: AiProviderUsage) {
        let _ = self.channel.send(usage);
    }
}
