//! Native notifications adapted onto the Tauri notification plugin.

use tauri::AppHandle;

use crate::agent_activity::notifications::NotificationFilter;
use crate::models::{AgentActivitySnapshot, AgentNotificationPreferences, Recommendation};
use crate::services::desktop_notifications::DesktopNotifications;

/// Delivers notifications through the plugin registered by the desktop shell.
pub struct TauriNotifications {
    app: AppHandle,
}

impl TauriNotifications {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl DesktopNotifications for TauriNotifications {
    fn request_permission(&self) -> Result<(), String> {
        crate::ai_control_center::notifications::request_permission_if_needed(&self.app)
    }

    fn emit_recommendations(&self, recommendations: &[Recommendation]) -> Vec<String> {
        crate::ai_control_center::notifications::emit_advisories(&self.app, recommendations)
    }

    fn emit_process_advisories(
        &self,
        snapshot: &AgentActivitySnapshot,
        preferences: &AgentNotificationPreferences,
        filter: &mut NotificationFilter,
    ) -> Vec<String> {
        crate::agent_activity::notifications::emit_process_advisories(
            &self.app,
            snapshot,
            preferences,
            filter,
        )
    }
}
