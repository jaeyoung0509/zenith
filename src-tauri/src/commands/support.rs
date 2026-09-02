use crate::models::AiUsageSnapshot;
use std::path::PathBuf;

pub(super) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Run native or filesystem work away from Tauri's command executor.
pub(super) async fn run_blocking<T, F>(work: F, context: &'static str) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| format!("{context}: {error}"))?
}

pub(super) fn user_home() -> Result<PathBuf, String> {
    crate::platform::NativePlatformPaths::new()
        .home()
        .ok_or_else(|| "User home directory is not available".to_string())
}

pub(super) fn usage_snapshot_matches_selection(
    snapshot: &AiUsageSnapshot,
    provider_ids: &[String],
) -> bool {
    snapshot.providers.len() == provider_ids.len()
        && snapshot
            .providers
            .iter()
            .zip(provider_ids)
            .all(|(provider, selected_id)| provider.id == *selected_id)
}
