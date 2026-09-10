use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

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

/// Acquires a mutex lock, recovering from poisoning if another thread panicked.
///
/// Use for disposable, observational, or cache state (e.g., last scan observations,
/// metric caches, read-only settings queries) where recovering the partially written
/// value or overwriting it is safe and prevents an isolated panic from wedging the app.
pub(crate) fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Acquires a mutex lock, failing closed with an informative error if poisoned.
///
/// Use for authority, mutation, or lifecycle-owned state (e.g., delete plans,
/// settings mutations) where an interrupted transaction indicates state corruption
/// and must not be silently resumed.
pub(crate) fn lock_or_state_error<'a, T>(
    mutex: &'a Mutex<T>,
    resource: &str,
) -> Result<MutexGuard<'a, T>, String> {
    mutex
        .lock()
        .map_err(|_| format!("{resource} state is unavailable due to an unexpected previous error"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn lock_recover_retrieves_guard_after_panic() {
        let data = Arc::new(Mutex::new(42));
        let data_clone = data.clone();

        let _ = thread::spawn(move || {
            let mut guard = data_clone.lock().unwrap();
            *guard = 99;
            panic!("intentional panic to poison mutex");
        })
        .join();

        assert!(data.is_poisoned());
        let recovered_guard = lock_recover(&data);
        assert_eq!(*recovered_guard, 99);
    }

    #[test]
    fn lock_or_state_error_fails_closed_after_panic() {
        let data = Arc::new(Mutex::new("critical_state"));
        let data_clone = data.clone();

        let _ = thread::spawn(move || {
            let mut guard = data_clone.lock().unwrap();
            *guard = "corrupted_state";
            panic!("intentional panic to poison mutex");
        })
        .join();

        assert!(data.is_poisoned());
        let result = lock_or_state_error(&data, "Delete plans");
        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Delete plans state is unavailable"));
    }

    #[test]
    fn lock_or_state_error_succeeds_when_not_poisoned() {
        let data = Mutex::new(123);
        let guard = lock_or_state_error(&data, "Settings").unwrap();
        assert_eq!(*guard, 123);
    }
}
