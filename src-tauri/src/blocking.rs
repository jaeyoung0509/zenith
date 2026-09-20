//! Running native and filesystem work away from Tauri's command executor.
//!
//! Commands and the application services both hand blocking work to this
//! module, so the panic-reporting contract is stated once: a worker that died
//! reports what it died with, through the redacting diagnostics sink.

/// The work function's own error type is preserved, so a command that states a
/// typed refusal does not have to flatten it into a string to survive the
/// worker boundary. A worker that died still reports through the caller's error
/// type, because a dead worker is that caller's internal failure.
pub(crate) async fn run_blocking<T, E, F>(work: F, context: &'static str) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| E::from(join_failure(context, error)))?
}

/// The error for a worker task that never returned its result.
///
/// The join error's `Display` carries the panic message the worker died with,
/// and `map_err(|_| "… panicked")` used to throw exactly that away: the log and
/// the user were left with the symptom and no cause. The raw message goes to the
/// diagnostics sink (which redacts credentials and masks paths); the value that
/// crosses IPC is sanitized the same way, so a panic payload cannot leak into
/// the interface what the log would have hidden.
pub(crate) fn join_failure(context: &str, error: impl std::fmt::Display) -> String {
    let message = format!("{context}: {error}");
    crate::diagnostics::log_error("worker", &message);
    crate::diagnostics::sanitize_log(&message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The error surfaced for a dead worker must carry what it died with: the
    /// previous `map_err(|_| "… panicked")` named only the symptom.
    #[test]
    fn join_failure_keeps_the_panic_payload() {
        let worker = tauri::async_runtime::spawn_blocking(|| panic!("cache index was truncated"));
        let error = tauri::async_runtime::block_on(worker).expect_err("the worker panicked");

        let message = join_failure("Large-file scan worker panicked", error);

        assert!(
            message.starts_with("Large-file scan worker panicked: "),
            "the caller's context must stay attached: {message}"
        );
        assert!(
            message.contains("cache index was truncated"),
            "the panic payload must survive: {message}"
        );
    }
}
