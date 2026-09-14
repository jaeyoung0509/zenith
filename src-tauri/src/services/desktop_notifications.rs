//! The desktop-notification port.
//!
//! Requesting the operating-system permission and delivering a native
//! notification are transport work: a service states that the user must be
//! told, and [`crate::events`] owns the plugin call. Keeping the port here lets
//! an application service be exercised without a Tauri runtime.

use crate::agent_activity::notifications::NotificationFilter;
use crate::models::{AgentActivitySnapshot, AgentNotificationPreferences, Recommendation};

pub trait DesktopNotifications: Send + Sync {
    /// Asks for the notification permission while the system is still
    /// undecided, and does nothing once the user has answered.
    fn request_permission(&self) -> Result<(), String>;

    /// Delivers eligible recommendations and returns one message per failure.
    fn emit_recommendations(&self, recommendations: &[Recommendation]) -> Vec<String>;

    /// Delivers "possibly inactive" advisories, honoring opt-in, the granted
    /// permission, and the caller's dedupe filter.
    fn emit_process_advisories(
        &self,
        snapshot: &AgentActivitySnapshot,
        preferences: &AgentNotificationPreferences,
        filter: &mut NotificationFilter,
    ) -> Vec<String>;
}
