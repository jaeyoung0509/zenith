use crate::models::{AgentNotificationPreferences, AttentionReason};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NotificationDedupeKey {
    pub session_id: String,
    pub event_kind: String,
    pub turn_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct NotificationFilter {
    sent_keys: HashSet<NotificationDedupeKey>,
}

impl NotificationFilter {
    pub fn should_notify(
        &mut self,
        prefs: &AgentNotificationPreferences,
        session_id: &str,
        event_kind: &str,
        turn_id: Option<&str>,
    ) -> bool {
        if !prefs.enabled {
            return false;
        }

        match event_kind {
            "turn_completed" => {
                if !prefs.notify_on_turn_completed {
                    return false;
                }
            }
            "waiting_for_user" => {
                if !prefs.notify_on_approval_or_input {
                    return false;
                }
            }
            "possibly_inactive" => {
                if !prefs.notify_on_possibly_inactive {
                    return false;
                }
            }
            _ => return false,
        }

        let key = NotificationDedupeKey {
            session_id: session_id.to_string(),
            event_kind: event_kind.to_string(),
            turn_id: turn_id.map(str::to_string),
        };

        if self.sent_keys.contains(&key) {
            return false;
        }
        self.sent_keys.insert(key);
        true
    }

    pub fn format_notification(
        prefs: &AgentNotificationPreferences,
        tool_name: &str,
        project_name: &str,
        event_kind: &str,
        attention_reason: Option<AttentionReason>,
    ) -> (String, String) {
        let title = tool_name.to_string();
        let target = if prefs.hide_project_basename {
            "an active project"
        } else {
            project_name
        };

        let body = match event_kind {
            "turn_completed" => format!("Turn completed in {target}."),
            "waiting_for_user" => match attention_reason {
                Some(AttentionReason::Approval) => format!("Needs approval in {target}."),
                Some(AttentionReason::Input) => format!("Waiting for input in {target}."),
                _ => format!("Waiting for user in {target}."),
            },
            "possibly_inactive" => format!("Possibly inactive in {target}."),
            _ => format!("Activity update in {target}."),
        };

        (title, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_opt_in_and_deduplicates_notifications() {
        let mut filter = NotificationFilter::default();
        let mut prefs = AgentNotificationPreferences {
            enabled: false,
            notify_on_turn_completed: true,
            notify_on_approval_or_input: true,
            notify_on_possibly_inactive: true,
            hide_project_basename: false,
            inactivity_threshold_minutes: 15,
        };

        // Disabled by default -> never notifies
        assert!(!filter.should_notify(&prefs, "s1", "turn_completed", Some("t1")));

        // Enable notifications
        prefs.enabled = true;
        assert!(filter.should_notify(&prefs, "s1", "turn_completed", Some("t1")));

        // Duplicate notification -> filtered out!
        assert!(!filter.should_notify(&prefs, "s1", "turn_completed", Some("t1")));

        // Next turn -> notifies
        assert!(filter.should_notify(&prefs, "s1", "turn_completed", Some("t2")));
    }

    #[test]
    fn privacy_formatting_never_exposes_full_paths_or_sensitive_strings() {
        let prefs = AgentNotificationPreferences {
            enabled: true,
            notify_on_turn_completed: true,
            notify_on_approval_or_input: true,
            notify_on_possibly_inactive: false,
            hide_project_basename: false,
            inactivity_threshold_minutes: 15,
        };

        let (title, body) = NotificationFilter::format_notification(
            &prefs,
            "Antigravity",
            "zenith",
            "waiting_for_user",
            Some(AttentionReason::Approval),
        );

        assert_eq!(title, "Antigravity");
        assert_eq!(body, "Needs approval in zenith.");
        assert!(!body.contains('/'));

        // With hide_project_basename enabled
        let mut private_prefs = prefs;
        private_prefs.hide_project_basename = true;
        let (_, hidden_body) = NotificationFilter::format_notification(
            &private_prefs,
            "Antigravity",
            "secret-project",
            "turn_completed",
            None,
        );
        assert_eq!(hidden_body, "Turn completed in an active project.");
        assert!(!hidden_body.contains("secret-project"));
    }
}
