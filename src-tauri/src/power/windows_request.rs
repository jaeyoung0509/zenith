//! Process-scoped Windows power requests with RAII cleanup on every exit path.
use crate::models::{AwakeBehavior, ZenithError};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired,
    PowerSetRequest, SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
    ES_SYSTEM_REQUIRED, POWER_REQUEST_TYPE,
};
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

#[derive(Debug)]
pub(super) enum WindowsPowerRequest {
    Request {
        handle: OwnedHandle,
        requests: &'static [POWER_REQUEST_TYPE],
        active_count: usize,
    },
    ThreadExecutionState {
        _previous: u32,
    },
}

impl WindowsPowerRequest {
    pub(super) fn acquire(behavior: AwakeBehavior, reason: &str) -> Result<Self, ZenithError> {
        if reason.contains('\0') {
            return Err(ZenithError::Io(
                "Power request reason contains a NUL character".into(),
            ));
        }
        let mut reason_u16: Vec<u16> = reason.encode_utf16().chain(Some(0)).collect();
        let context = REASON_CONTEXT {
            Version: 0,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: reason_u16.as_mut_ptr(),
            },
        };
        // The reason buffer and context remain valid for the complete FFI call.
        let raw = unsafe { PowerCreateRequest(&context) };
        if !raw.is_null() && raw != INVALID_HANDLE_VALUE {
            let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
            let requests = match behavior {
                AwakeBehavior::PreventSystemSleep => &[PowerRequestSystemRequired][..],
                AwakeBehavior::KeepDisplayAwake => {
                    &[PowerRequestSystemRequired, PowerRequestDisplayRequired][..]
                }
            };
            let mut active_count = 0;
            let mut success = true;
            for (index, &kind) in requests.iter().enumerate() {
                if unsafe { PowerSetRequest(handle.as_raw_handle(), kind) } == 0 {
                    success = false;
                    break;
                }
                active_count = index + 1;
            }
            if success {
                return Ok(WindowsPowerRequest::Request {
                    handle,
                    requests: match behavior {
                        AwakeBehavior::PreventSystemSleep => &[PowerRequestSystemRequired],
                        AwakeBehavior::KeepDisplayAwake => {
                            &[PowerRequestSystemRequired, PowerRequestDisplayRequired]
                        }
                    },
                    active_count,
                });
            }
        }

        // Fallback to SetThreadExecutionState if PowerCreateRequest / PowerSetRequest is unavailable.
        let flags = match behavior {
            AwakeBehavior::PreventSystemSleep => ES_CONTINUOUS | ES_SYSTEM_REQUIRED,
            AwakeBehavior::KeepDisplayAwake => {
                ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
            }
        };
        let previous = unsafe { SetThreadExecutionState(flags) };
        if previous == 0 {
            return Err(last_error("SetThreadExecutionState"));
        }
        Ok(WindowsPowerRequest::ThreadExecutionState {
            _previous: previous,
        })
    }
}

impl Drop for WindowsPowerRequest {
    fn drop(&mut self) {
        match self {
            WindowsPowerRequest::Request {
                handle,
                requests,
                active_count,
            } => {
                for &kind in requests[..*active_count].iter().rev() {
                    unsafe {
                        PowerClearRequest(handle.as_raw_handle(), kind);
                    }
                }
            }
            WindowsPowerRequest::ThreadExecutionState { .. } => unsafe {
                SetThreadExecutionState(ES_CONTINUOUS);
            },
        }
    }
}

fn last_error(operation: &str) -> ZenithError {
    ZenithError::Io(format!(
        "{operation} failed: {}",
        std::io::Error::last_os_error()
    ))
}
