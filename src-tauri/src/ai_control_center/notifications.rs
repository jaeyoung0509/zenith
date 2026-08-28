use crate::models::{Recommendation, RecommendationKind};
use tauri::plugin::PermissionState;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

pub fn request_permission_if_needed(app: &AppHandle) -> Result<(), String> {
    let notification = app.notification();
    if notification
        .permission_state()
        .map_err(|error| error.to_string())?
        == PermissionState::Prompt
    {
        notification
            .request_permission()
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn emit_advisories(app: &AppHandle, recommendations: &[Recommendation]) -> Vec<String> {
    let eligible = recommendations
        .iter()
        .filter(|item| is_notification_kind(item.kind))
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return Vec::new();
    }
    let notification = app.notification();
    if notification.permission_state().ok() != Some(PermissionState::Granted) {
        return vec!["Notification permission is not granted".into()];
    }
    eligible
        .into_iter()
        .filter_map(|item| {
            notification
                .builder()
                .title(&item.title)
                .body(&item.message)
                .show()
                .err()
                .map(|error| format!("Native notification failed: {error}"))
        })
        .collect()
}

fn is_notification_kind(kind: RecommendationKind) -> bool {
    matches!(
        kind,
        RecommendationKind::Battery
            | RecommendationKind::Memory
            | RecommendationKind::SessionCompleted
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_notification_kinds_are_eligible() {
        assert!(is_notification_kind(RecommendationKind::Battery));
        assert!(is_notification_kind(RecommendationKind::Memory));
        assert!(is_notification_kind(RecommendationKind::SessionCompleted));
        assert!(!is_notification_kind(RecommendationKind::DevelopmentPort));
        assert!(!is_notification_kind(RecommendationKind::CleanupReview));
        assert!(!is_notification_kind(RecommendationKind::OrphanProcess));
    }
}
