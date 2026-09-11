//! Shared process-termination mechanisms with an ordered fallback chain.
//!
//! Callers remain responsible for ownership/identity verification before
//! invoking these functions; this module owns only how a stop request is
//! delivered on each platform. Newer Windows APIs are reached through
//! `windows-sys` entry points so the process still loads where a call is
//! absent, and a mechanism is only reported unavailable after it was tried.

/// How the caller wants the target process stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationMode {
    /// Ask the process to exit. May be unavailable for a target without a
    /// window or console, in which case an explicit error is returned.
    Graceful,
    /// Force termination. Always attempted last by callers that fall back.
    Force,
}

/// Outcome of a graceful stop request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GracefulStopOutcome {
    /// A platform graceful mechanism delivered the request.
    Delivered,
    /// No window or console control mechanism exists for this target. Callers
    /// that intend to stop the process must re-verify identity and force.
    Unavailable,
}

/// Requests a graceful stop without ever forcing. Callers own the fallback so
/// force termination can only happen after a fresh identity verification.
#[cfg(unix)]
pub fn request_graceful_stop(pid: u32) -> Result<GracefulStopOutcome, String> {
    let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if result == 0 {
        return Ok(GracefulStopOutcome::Delivered);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        // The process already exited; the desired end state is reached.
        return Ok(GracefulStopOutcome::Delivered);
    }
    Err(format!("Could not signal process {pid}: {error}"))
}

#[cfg(target_os = "windows")]
pub fn request_graceful_stop(pid: u32) -> Result<GracefulStopOutcome, String> {
    if post_close_to_process(pid) {
        return Ok(GracefulStopOutcome::Delivered);
    }
    if send_console_break(pid) {
        return Ok(GracefulStopOutcome::Delivered);
    }
    Ok(GracefulStopOutcome::Unavailable)
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn request_graceful_stop(pid: u32) -> Result<GracefulStopOutcome, String> {
    let _ = pid;
    Err("Process termination is unavailable on this platform.".to_string())
}

#[cfg(unix)]
pub fn terminate_process(pid: u32, mode: TerminationMode) -> Result<(), String> {
    match mode {
        TerminationMode::Graceful => request_graceful_stop(pid).map(|_| ()),
        TerminationMode::Force => {
            let result = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            if result == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                // The process already exited; the desired end state is reached.
                return Ok(());
            }
            Err(format!("Could not signal process {pid}: {error}"))
        }
    }
}

#[cfg(target_os = "windows")]
pub fn terminate_process(pid: u32, mode: TerminationMode) -> Result<(), String> {
    match mode {
        TerminationMode::Graceful => match request_graceful_stop(pid)? {
            GracefulStopOutcome::Delivered => Ok(()),
            GracefulStopOutcome::Unavailable => Err(
                "No window or console control mechanism is available for this process.".to_string(),
            ),
        },
        TerminationMode::Force => force_terminate(pid),
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
pub fn terminate_process(pid: u32, _mode: TerminationMode) -> Result<(), String> {
    let _ = pid;
    Err("Process termination is unavailable on this platform.".to_string())
}

/// Posts `WM_CLOSE` to every top-level window owned by the target process.
#[cfg(target_os = "windows")]
fn post_close_to_process(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };

    struct Search {
        pid: u32,
        posted: bool,
    }

    unsafe extern "system" fn callback(
        hwnd: windows_sys::Win32::Foundation::HWND,
        lparam: LPARAM,
    ) -> BOOL {
        let search = &mut *(lparam as *mut Search);
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == search.pid && PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 {
            search.posted = true;
            return 0; // Stop enumeration after the first delivered close request.
        }
        1
    }

    let mut search = Search { pid, posted: false };
    unsafe {
        EnumWindows(Some(callback), &mut search as *mut Search as LPARAM);
    }
    search.posted
}

/// Delivers `CTRL_BREAK_EVENT` to the target's console process group.
#[cfg(target_os = "windows")]
fn send_console_break(pid: u32) -> bool {
    use windows_sys::Win32::System::Console::{
        AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
        CTRL_BREAK_EVENT,
    };

    unsafe {
        // Zenith is a GUI process without an owned console, but detach first in
        // case a debugger attached one; failures are harmless.
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return false;
        }
        // Ignore the break inside Zenith while the event is delivered.
        SetConsoleCtrlHandler(None, 1);
        let delivered = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) != 0;
        SetConsoleCtrlHandler(None, 0);
        FreeConsole();
        delivered
    }
}

#[cfg(target_os = "windows")]
fn force_terminate(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return match GetLastError() {
                ERROR_ACCESS_DENIED => Err("Access denied terminating process".to_string()),
                // Any other failure for a non-existent process means the desired
                // end state is already reached.
                _ => Ok(()),
            };
        }
        let success = TerminateProcess(handle, 1);
        CloseHandle(handle);
        if success == 0 {
            Err("Failed to terminate process on Windows".to_string())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GracefulStopOutcome, TerminationMode};

    #[test]
    #[cfg(unix)]
    fn signal_to_missing_process_is_treated_as_already_stopped() {
        // An unused PID yields ESRCH, which the helper reports as success.
        let missing = u32::MAX - 1;
        assert!(super::terminate_process(missing, TerminationMode::Force).is_ok());
        assert!(super::terminate_process(missing, TerminationMode::Graceful).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn graceful_request_to_missing_process_is_delivered() {
        let missing = u32::MAX - 1;
        assert_eq!(
            super::request_graceful_stop(missing).unwrap(),
            GracefulStopOutcome::Delivered
        );
    }

    #[test]
    fn termination_modes_are_distinct() {
        assert_ne!(TerminationMode::Graceful, TerminationMode::Force);
    }

    #[test]
    fn graceful_outcomes_are_distinct() {
        assert_ne!(
            GracefulStopOutcome::Delivered,
            GracefulStopOutcome::Unavailable
        );
    }
}
