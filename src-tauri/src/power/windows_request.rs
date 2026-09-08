//! Process-scoped Windows power requests with RAII cleanup on every exit path.
use crate::models::{AwakeBehavior, ZenithError};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired,
    PowerSetRequest, POWER_REQUEST_TYPE,
};
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

#[derive(Debug)]
pub(super) struct WindowsPowerRequest {
    handle: OwnedHandle,
    requests: &'static [POWER_REQUEST_TYPE],
    active_count: usize,
}

impl WindowsPowerRequest {
    pub(super) fn acquire(behavior: AwakeBehavior, reason: &str) -> Result<Self, ZenithError> {
        if reason.contains('\0') {
            return Err(ZenithError::Io(
                "Power request reason contains a NUL character".into(),
            ));
        }
        let mut reason: Vec<u16> = reason.encode_utf16().chain(Some(0)).collect();
        let context = REASON_CONTEXT {
            Version: 0,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: reason.as_mut_ptr(),
            },
        };
        // The reason buffer and context remain valid for the complete FFI call.
        let raw = unsafe { PowerCreateRequest(&context) };
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(last_error("PowerCreateRequest"));
        }
        let mut request = Self {
            // SAFETY: PowerCreateRequest returned a fresh, valid owned handle.
            handle: unsafe { OwnedHandle::from_raw_handle(raw) },
            requests: match behavior {
                AwakeBehavior::PreventSystemSleep => &[PowerRequestSystemRequired],
                AwakeBehavior::KeepDisplayAwake => {
                    &[PowerRequestSystemRequired, PowerRequestDisplayRequired]
                }
            },
            active_count: 0,
        };
        for (index, &kind) in request.requests.iter().enumerate() {
            // The owned handle remains alive throughout this call.
            if unsafe { PowerSetRequest(request.handle.as_raw_handle(), kind) } == 0 {
                return Err(last_error("PowerSetRequest"));
            }
            request.active_count = index + 1;
        }
        Ok(request)
    }
}

impl Drop for WindowsPowerRequest {
    fn drop(&mut self) {
        for &kind in self.requests[..self.active_count].iter().rev() {
            // Clear only requests that succeeded, including partial acquisition.
            unsafe {
                PowerClearRequest(self.handle.as_raw_handle(), kind);
            }
        }
        // OwnedHandle closes the handle after this destructor, on any thread.
    }
}

fn last_error(operation: &str) -> ZenithError {
    ZenithError::Io(format!(
        "{operation} failed: {}",
        std::io::Error::last_os_error()
    ))
}
