use crate::models::{AwakeBehavior, ZenithError};

#[cfg(target_os = "macos")]
mod macos_iokit {
    use std::ffi::c_void;

    pub type IOPMAssertionID = u32;
    pub type IOReturn = i32;

    pub const K_IOPMASSERTION_LEVEL_ON: u32 = 255;
    pub const K_IORETURN_SUCCESS: i32 = 0;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOPMAssertionCreateWithName(
            assertion_type: *const c_void,
            assertion_level: u32,
            assertion_name: *const c_void,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;

        pub fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const libc::c_char,
            encoding: u32,
        ) -> *const c_void;

        pub fn CFRelease(cf: *const c_void);
    }

    pub const K_CFSTRING_ENCODING_UTF8: u32 = 0x08000100;
}

#[derive(Debug)]
pub struct PowerAssertion {
    #[cfg(target_os = "macos")]
    id: macos_iokit::IOPMAssertionID,
    #[cfg(target_os = "windows")]
    // Retained solely for RAII lifetime ownership; mocks do not acquire an OS request.
    _request: Option<super::windows_request::WindowsPowerRequest>,
    pub behavior: AwakeBehavior,
}

impl PowerAssertion {
    /// Creates a macOS power assertion using IOKit, preventing system sleep or keeping display awake.
    pub fn acquire(behavior: AwakeBehavior, reason: &str) -> Result<Self, ZenithError> {
        #[cfg(target_os = "macos")]
        {
            use macos_iokit::*;
            use std::ffi::CString;
            use std::ptr;

            let type_str = match behavior {
                AwakeBehavior::PreventSystemSleep => "PreventUserIdleSystemSleep",
                AwakeBehavior::KeepDisplayAwake => "PreventUserIdleDisplaySleep",
            };

            let c_type = CString::new(type_str).map_err(|e| ZenithError::Io(e.to_string()))?;
            let c_reason = CString::new(reason).map_err(|e| ZenithError::Io(e.to_string()))?;

            unsafe {
                let cf_type = CFStringCreateWithCString(
                    ptr::null(),
                    c_type.as_ptr(),
                    K_CFSTRING_ENCODING_UTF8,
                );
                let cf_reason = CFStringCreateWithCString(
                    ptr::null(),
                    c_reason.as_ptr(),
                    K_CFSTRING_ENCODING_UTF8,
                );

                let mut assertion_id: IOPMAssertionID = 0;
                let status = IOPMAssertionCreateWithName(
                    cf_type,
                    K_IOPMASSERTION_LEVEL_ON,
                    cf_reason,
                    &mut assertion_id,
                );

                if !cf_type.is_null() {
                    CFRelease(cf_type);
                }
                if !cf_reason.is_null() {
                    CFRelease(cf_reason);
                }

                if status == K_IORETURN_SUCCESS {
                    Ok(PowerAssertion {
                        id: assertion_id,
                        behavior,
                    })
                } else {
                    Err(ZenithError::Io(format!(
                        "IOKit power assertion failed with return code: {}",
                        status
                    )))
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            let request = super::windows_request::WindowsPowerRequest::acquire(behavior, reason)?;
            Ok(Self {
                _request: Some(request),
                behavior,
            })
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (behavior, reason);
            Err(ZenithError::ToolUnavailable(
                "Keep Awake is unavailable on this platform".to_string(),
            ))
        }
    }

    #[cfg(test)]
    pub(crate) fn mock(behavior: AwakeBehavior) -> Self {
        Self {
            #[cfg(target_os = "macos")]
            id: 1,
            #[cfg(target_os = "windows")]
            _request: None,
            behavior,
        }
    }
}

impl Drop for PowerAssertion {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            use macos_iokit::*;
            unsafe {
                let status = IOPMAssertionRelease(self.id);
                if status != K_IORETURN_SUCCESS {
                    eprintln!(
                        "Failed to release power assertion {}: code {}",
                        self.id, status
                    );
                }
            }
        }
    }
}

#[cfg(all(test, not(any(target_os = "macos", target_os = "windows"))))]
mod unsupported_tests {
    use super::PowerAssertion;
    use crate::models::{AwakeBehavior, ZenithError};

    #[test]
    fn native_assertion_fails_closed_when_no_adapter_exists() {
        let result = PowerAssertion::acquire(AwakeBehavior::PreventSystemSleep, "test");
        assert!(matches!(
            result,
            Err(ZenithError::ToolUnavailable(message)) if message.contains("unavailable")
        ));
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod native_tests {
    use super::PowerAssertion;
    use crate::models::AwakeBehavior;

    #[test]
    fn native_assertion_succeeds_with_adapter() {
        let result = PowerAssertion::acquire(AwakeBehavior::PreventSystemSleep, "test");
        assert!(result.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_request_handle_survives_across_threads() {
        let assertion =
            PowerAssertion::acquire(AwakeBehavior::PreventSystemSleep, "Zenith Worker Test")
                .unwrap();
        assert!(assertion._request.is_some());
        let handle = std::thread::spawn(move || {
            assert!(assertion._request.is_some());
            drop(assertion);
        });
        handle.join().unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_request_lifecycle_and_behavior_changes() {
        let a1 = PowerAssertion::acquire(AwakeBehavior::PreventSystemSleep, "Test Sleep").unwrap();
        assert!(a1._request.is_some());
        drop(a1);
        let a2 = PowerAssertion::acquire(AwakeBehavior::KeepDisplayAwake, "Test Display").unwrap();
        assert!(a2._request.is_some());
        drop(a2);
    }
}

pub trait PowerAssertionProvider: Send + Sync {
    fn acquire(&self, behavior: AwakeBehavior, reason: &str)
        -> Result<PowerAssertion, ZenithError>;
}

#[derive(Default, Clone, Copy)]
pub struct NativeAssertionProvider;

impl NativeAssertionProvider {
    pub fn new() -> Self {
        Self
    }
}

impl PowerAssertionProvider for NativeAssertionProvider {
    fn acquire(
        &self,
        behavior: AwakeBehavior,
        reason: &str,
    ) -> Result<PowerAssertion, ZenithError> {
        PowerAssertion::acquire(behavior, reason)
    }
}
