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
    pub(crate) handle: windows_sys::Win32::Foundation::HANDLE,
    pub behavior: AwakeBehavior,
}

#[cfg(target_os = "windows")]
unsafe impl Send for PowerAssertion {}
#[cfg(target_os = "windows")]
unsafe impl Sync for PowerAssertion {}

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
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Power::*;
            use windows_sys::Win32::System::Threading::{
                POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
            };

            let reason_wide: Vec<u16> = reason.encode_utf16().chain(std::iter::once(0)).collect();
            let mut context = REASON_CONTEXT {
                Version: 0,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: reason_wide.as_ptr() as *mut u16,
                },
            };
            let handle = unsafe { PowerCreateRequest(&context) };
            if handle.is_null() || handle == -1isize as _ {
                return Err(ZenithError::Io(
                    "PowerCreateRequest failed on Windows".to_string(),
                ));
            }

            let (sys_ok, disp_ok) = match behavior {
                AwakeBehavior::PreventSystemSleep => {
                    let ok = unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) };
                    (ok != 0, true)
                }
                AwakeBehavior::KeepDisplayAwake => {
                    let s_ok = unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) };
                    let d_ok = unsafe { PowerSetRequest(handle, PowerRequestDisplayRequired) };
                    (s_ok != 0, d_ok != 0)
                }
            };

            if !sys_ok || !disp_ok {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(ZenithError::Io("PowerSetRequest failed on Windows".to_string()));
            }

            Ok(PowerAssertion { handle, behavior })
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
            handle: std::ptr::null_mut(),
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

        #[cfg(target_os = "windows")]
        {
            if !self.handle.is_null() {
                use windows_sys::Win32::Foundation::CloseHandle;
                use windows_sys::Win32::System::Power::*;
                unsafe {
                    match self.behavior {
                        AwakeBehavior::PreventSystemSleep => {
                            PowerClearRequest(self.handle, PowerRequestSystemRequired);
                        }
                        AwakeBehavior::KeepDisplayAwake => {
                            PowerClearRequest(self.handle, PowerRequestDisplayRequired);
                            PowerClearRequest(self.handle, PowerRequestSystemRequired);
                        }
                    }
                    CloseHandle(self.handle);
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
        assert!(!assertion.handle.is_null());
        let handle = std::thread::spawn(move || {
            assert!(!assertion.handle.is_null());
            drop(assertion);
        });
        handle.join().unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_power_request_lifecycle_and_behavior_changes() {
        let a1 = PowerAssertion::acquire(AwakeBehavior::PreventSystemSleep, "Test Sleep").unwrap();
        assert!(!a1.handle.is_null());
        drop(a1);
        let a2 = PowerAssertion::acquire(AwakeBehavior::KeepDisplayAwake, "Test Display").unwrap();
        assert!(!a2.handle.is_null());
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
