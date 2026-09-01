use crate::models::PowerSourceType;

pub trait PowerSourceProvider: Send + Sync {
    fn current_power_source(&self) -> PowerSourceType;
}

#[derive(Default, Clone, Copy)]
pub struct SystemPowerSource;

impl SystemPowerSource {
    pub fn new() -> Self {
        Self
    }
}

impl PowerSourceProvider for SystemPowerSource {
    fn current_power_source(&self) -> PowerSourceType {
        #[cfg(target_os = "macos")]
        {
            mod macos_ps {
                use std::ffi::c_void;

                pub const K_CFSTRING_ENCODING_UTF8: u32 = 0x08000100;

                #[link(name = "IOKit", kind = "framework")]
                extern "C" {
                    pub fn IOPSCopyPowerSourcesInfo() -> *const c_void;
                    pub fn IOPSGetProvidingPowerSourceType(
                        snapshot: *const c_void,
                    ) -> *const c_void;
                }

                #[link(name = "CoreFoundation", kind = "framework")]
                extern "C" {
                    pub fn CFStringGetCString(
                        the_string: *const c_void,
                        buffer: *mut libc::c_char,
                        buffer_size: libc::c_long,
                        encoding: u32,
                    ) -> bool;
                    pub fn CFRelease(cf: *const c_void);
                }
            }

            unsafe {
                let snapshot = macos_ps::IOPSCopyPowerSourcesInfo();
                if !snapshot.is_null() {
                    let ps_type = macos_ps::IOPSGetProvidingPowerSourceType(snapshot);
                    if !ps_type.is_null() {
                        let mut buf = [0u8; 128];
                        let success = macos_ps::CFStringGetCString(
                            ps_type,
                            buf.as_mut_ptr() as *mut libc::c_char,
                            buf.len() as libc::c_long,
                            macos_ps::K_CFSTRING_ENCODING_UTF8,
                        );

                        macos_ps::CFRelease(snapshot);

                        if success {
                            let str_slice =
                                std::ffi::CStr::from_ptr(buf.as_ptr() as *const libc::c_char)
                                    .to_string_lossy();
                            if str_slice.contains("AC Power") {
                                return PowerSourceType::Ac;
                            } else if str_slice.contains("Battery") {
                                return PowerSourceType::Battery;
                            }
                        }
                    } else {
                        macos_ps::CFRelease(snapshot);
                    }
                }
            }

            fallback_pmset_power_source()
        }

        #[cfg(not(target_os = "macos"))]
        {
            PowerSourceType::Unknown
        }
    }
}

#[cfg(target_os = "macos")]
fn fallback_pmset_power_source() -> PowerSourceType {
    let mut cmd = std::process::Command::new("pmset");
    cmd.args(["-g", "batt"]);
    if let Ok(output) = crate::tooling::run_with_timeout(cmd, std::time::Duration::from_secs(3)) {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains("AC Power") {
            return PowerSourceType::Ac;
        } else if text.contains("Battery Power") {
            return PowerSourceType::Battery;
        }
    }
    PowerSourceType::Unknown
}

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct MockPowerSource {
    source: PowerSourceType,
    query_count: Arc<AtomicUsize>,
}

impl MockPowerSource {
    pub fn new(source: PowerSourceType) -> Self {
        Self {
            source,
            query_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn set_source(&mut self, source: PowerSourceType) {
        self.source = source;
    }

    pub fn query_count(&self) -> usize {
        self.query_count.load(Ordering::SeqCst)
    }
}

impl PowerSourceProvider for MockPowerSource {
    fn current_power_source(&self) -> PowerSourceType {
        self.query_count.fetch_add(1, Ordering::SeqCst);
        self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_power_source_returns_configured_source() {
        let mut mock = MockPowerSource::new(PowerSourceType::Battery);
        assert_eq!(mock.current_power_source(), PowerSourceType::Battery);

        mock.set_source(PowerSourceType::Ac);
        assert_eq!(mock.current_power_source(), PowerSourceType::Ac);
    }

    #[test]
    fn system_power_source_returns_valid_variant() {
        let provider = SystemPowerSource::new();
        let source = provider.current_power_source();
        assert!(matches!(
            source,
            PowerSourceType::Ac | PowerSourceType::Battery | PowerSourceType::Unknown
        ));

        #[cfg(not(target_os = "macos"))]
        assert_eq!(source, PowerSourceType::Unknown);
    }
}
