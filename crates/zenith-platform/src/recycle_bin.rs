//! The operating system's Recycle Bin, behind a port.
//!
//! This is the raw native primitive, and it is deliberately *not* an authority
//! boundary: a trait any crate can implement cannot be a capability. The
//! reviewed action — which platform offers it, what it states before running,
//! and whether its postcondition holds — lives in the provider that calls this
//! port, exactly as moving a path to the Trash lives in `TrashExecutor`.
//!
//! Two calls are all the platform exposes through a supported interface:
//!
//! * [`RecycleBinBackend::observe`] reads the size and item count the shell
//!   reports for the current account across every volume. It is the only
//!   measurement a scan may use, because the Recycle Bin's on-disk layout
//!   (`$Recycle.Bin` per volume, indexed per security identifier) is a shell
//!   implementation detail that must never be walked or deleted directly.
//! * [`RecycleBinBackend::empty`] asks the shell to empty it. The call is
//!   reported separately from its verification on purpose: an interface that
//!   returns an error while the bin is in fact empty exists in practice, so the
//!   caller re-observes and reports what it measured rather than what the
//!   return code suggested.

/// What one observation of the Recycle Bin reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecycleBinObservation {
    /// Bytes the Recycle Bin holds, across every volume the account can see.
    pub bytes: u64,
    /// How many items it holds.
    pub item_count: u64,
}

/// Why an operation on the Recycle Bin could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecycleBinError {
    /// No adapter for the Recycle Bin exists on this platform or in this
    /// build. This is a stated answer, not a failure: nothing can be retried
    /// into existence.
    Unsupported,
    /// The interface exists and refused or failed. The message carries the
    /// platform's own words.
    Failed(String),
}

impl RecycleBinError {
    /// The refusal phrased for a message the user reads.
    pub fn describe(&self) -> String {
        match self {
            Self::Unsupported => {
                "The Recycle Bin can only be emptied through the interface this platform provides, and this build has no adapter for it.".to_string()
            }
            Self::Failed(message) => message.clone(),
        }
    }
}

/// Port for reading and emptying the operating system's Recycle Bin.
pub trait RecycleBinBackend: Send + Sync {
    /// Reads what the Recycle Bin currently holds.
    fn observe(&self) -> Result<RecycleBinObservation, RecycleBinError>;

    /// Asks the operating system to empty the Recycle Bin.
    ///
    /// A successful return states that the interface accepted the request; it
    /// is not a measurement. The caller verifies by observing again.
    fn empty(&self) -> Result<(), RecycleBinError>;
}

/// Native implementation: the Windows shell's Recycle Bin interface.
///
/// On a platform whose shell exposes no such interface the port answers
/// [`RecycleBinError::Unsupported`] rather than synthesizing a result.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeRecycleBinBackend;

impl RecycleBinBackend for NativeRecycleBinBackend {
    fn observe(&self) -> Result<RecycleBinObservation, RecycleBinError> {
        platform::observe()
    }

    fn empty(&self) -> Result<(), RecycleBinError> {
        platform::empty()
    }
}

/// In-memory Recycle Bin backend for tests, recording operations without
/// touching a real Recycle Bin.
///
/// The knobs are the states a real machine can be in and a test must be able to
/// state: a bin holding bytes, a platform with no adapter, an observation the
/// shell refuses, an empty attempt that fails while leaving the bin untouched,
/// and an empty attempt that succeeds without removing everything. A poisoned
/// lock here means a test panicked while recording, and the recorded state is
/// disposable, so recovering it keeps that failure visible.
#[derive(Debug)]
pub struct MockRecycleBinBackend {
    state: std::sync::Mutex<MockState>,
}

#[derive(Debug)]
struct MockState {
    bytes: u64,
    item_count: u64,
    supported: bool,
    observe_error: Option<RecycleBinError>,
    empty_error: Option<RecycleBinError>,
    bytes_after_empty: u64,
    item_count_after_empty: u64,
    observe_calls: usize,
    empty_calls: usize,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            bytes: 0,
            item_count: 0,
            supported: true,
            observe_error: None,
            empty_error: None,
            bytes_after_empty: 0,
            item_count_after_empty: 0,
            observe_calls: 0,
            empty_calls: 0,
        }
    }
}

impl MockRecycleBinBackend {
    /// A bin holding `bytes` across `item_count` items, which a successful
    /// empty removes entirely.
    pub fn holding(bytes: u64, item_count: u64) -> Self {
        Self {
            state: std::sync::Mutex::new(MockState {
                bytes,
                item_count,
                ..MockState::default()
            }),
        }
    }

    /// A platform with no Recycle Bin adapter.
    pub fn unsupported() -> Self {
        Self {
            state: std::sync::Mutex::new(MockState {
                supported: false,
                ..MockState::default()
            }),
        }
    }

    /// A backend whose observations are refused with `error`.
    pub fn failing_observe(error: RecycleBinError) -> Self {
        Self {
            state: std::sync::Mutex::new(MockState {
                observe_error: Some(error),
                ..MockState::default()
            }),
        }
    }

    /// An empty attempt that is refused with `error`, after which the bin holds
    /// `bytes_after`.
    ///
    /// `bytes_after` equal to the current size is a refusal that removed
    /// nothing; `bytes_after` of zero is a shell that reported an error while
    /// the bin it owns is in fact empty, which happens in practice for an
    /// already-empty bin.
    pub fn with_empty_error(self, error: RecycleBinError, bytes_after: u64) -> Self {
        let mut state = self.lock();
        state.empty_error = Some(error);
        state.bytes_after_empty = bytes_after;
        state.item_count_after_empty = u64::from(bytes_after > 0);
        // The guard borrows `self`, so it is released before the value is
        // handed back to the caller.
        drop(state);
        self
    }

    /// An empty attempt that succeeds without removing everything: the bin
    /// holds `bytes_after` afterwards.
    pub fn with_empty_remaining(self, bytes_after: u64) -> Self {
        let mut state = self.lock();
        state.bytes_after_empty = bytes_after;
        state.item_count_after_empty = u64::from(bytes_after > 0);
        drop(state);
        self
    }

    /// How many times the backend was asked to observe.
    pub fn observe_calls(&self) -> usize {
        self.lock().observe_calls
    }

    /// How many times the backend was asked to empty.
    pub fn empty_calls(&self) -> usize {
        self.lock().empty_calls
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl RecycleBinBackend for MockRecycleBinBackend {
    fn observe(&self) -> Result<RecycleBinObservation, RecycleBinError> {
        let mut state = self.lock();
        state.observe_calls += 1;
        if !state.supported {
            return Err(RecycleBinError::Unsupported);
        }
        if let Some(error) = state.observe_error.clone() {
            return Err(error);
        }
        Ok(RecycleBinObservation {
            bytes: state.bytes,
            item_count: state.item_count,
        })
    }

    fn empty(&self) -> Result<(), RecycleBinError> {
        let mut state = self.lock();
        state.empty_calls += 1;
        if !state.supported {
            return Err(RecycleBinError::Unsupported);
        }
        let error = state.empty_error.clone();
        state.bytes = state.bytes_after_empty;
        state.item_count = state.item_count_after_empty;
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{RecycleBinError, RecycleBinObservation};
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::UI::Shell::{
        SHEmptyRecycleBinW, SHQueryRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
        SHERB_NOSOUND, SHQUERYRBINFO,
    };

    /// Runs one Recycle Bin call on a thread that owns a single-threaded
    /// apartment.
    ///
    /// The Recycle Bin is a shell object reached through COM, and the callers
    /// are worker threads whose apartment nobody selected. Rather than guess at
    /// the apartment the shell needs, the call gets a fresh STA thread of its
    /// own: it runs once, answers once, and the thread exits, which costs less
    /// than the ambiguity it removes.
    fn on_shell_thread<T: Send + 'static>(
        call: impl FnOnce() -> Result<T, RecycleBinError> + Send + 'static,
    ) -> Result<T, RecycleBinError> {
        let worker = std::thread::Builder::new()
            .name("zenith-recycle-bin".to_string())
            .spawn(move || {
                let status =
                    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
                // `S_OK` and `S_FALSE` both mean this thread now holds the
                // apartment and must release it; a failure means the thread
                // did not get one, and the call below reports what the shell
                // says about that.
                let holds_apartment = status >= 0;
                let result = call();
                if holds_apartment {
                    unsafe { CoUninitialize() };
                }
                result
            })
            .map_err(|error| {
                RecycleBinError::Failed(format!(
                    "Could not start the Recycle Bin worker thread: {error}"
                ))
            })?;
        worker.join().map_err(|_| {
            RecycleBinError::Failed("The Recycle Bin worker thread panicked".to_string())
        })?
    }

    /// The shell's own failure code, phrased for a message.
    ///
    /// The raw HRESULT is kept: it is the only part of a shell failure that is
    /// stable across interface locales, and it is what a bug report needs.
    fn shell_failure(call: &str, status: i32) -> RecycleBinError {
        RecycleBinError::Failed(format!("{call} failed with 0x{:08X}", status as u32))
    }

    pub fn observe() -> Result<RecycleBinObservation, RecycleBinError> {
        on_shell_thread(|| {
            let mut info = SHQUERYRBINFO {
                cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
                i64Size: 0,
                i64NumItems: 0,
            };
            // A null root asks for every Recycle Bin on every drive, which is
            // the scope of the reviewed action: the Recycle Bin, not one
            // volume's bin.
            let status = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
            if status < 0 {
                return Err(shell_failure("SHQueryRecycleBin", status));
            }
            Ok(RecycleBinObservation {
                // A negative size is not a measurement; clamping keeps a
                // nonsensical answer from entering a total as a huge number.
                bytes: u64::try_from(info.i64Size).unwrap_or(0),
                item_count: u64::try_from(info.i64NumItems).unwrap_or(0),
            })
        })
    }

    pub fn empty() -> Result<(), RecycleBinError> {
        on_shell_thread(|| {
            // Zenith asks the user before this runs, so the shell's own
            // confirmation and progress UI are suppressed: on a thread with no
            // window they would block the call, and the confirmation that
            // authorizes the action is the reviewed plan, not a dialog the
            // shell draws. `SHERB_NOSOUND` matches that.
            let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
            let status =
                unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), flags) };
            if status < 0 {
                return Err(shell_failure("SHEmptyRecycleBin", status));
            }
            Ok(())
        })
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::{RecycleBinError, RecycleBinObservation};

    /// No adapter on this platform: the shell interface this port wraps is
    /// Windows-only, and the Trash on other platforms is emptied by the file
    /// manager that owns it.
    pub fn observe() -> Result<RecycleBinObservation, RecycleBinError> {
        Err(RecycleBinError::Unsupported)
    }

    pub fn empty() -> Result<(), RecycleBinError> {
        Err(RecycleBinError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MockRecycleBinBackend, NativeRecycleBinBackend, RecycleBinBackend, RecycleBinError,
        RecycleBinObservation,
    };

    /// The mock answers the states a real machine can be in, and it records
    /// that each call happened exactly once — a caller that claims to verify an
    /// action has to have observed again.
    #[test]
    fn the_mock_answers_each_state_and_counts_its_calls() {
        let holding = MockRecycleBinBackend::holding(4_096, 3);
        assert_eq!(
            holding.observe().expect("a recording backend answers"),
            RecycleBinObservation {
                bytes: 4_096,
                item_count: 3
            }
        );
        holding.empty().expect("an empty attempt that succeeds");
        assert_eq!(holding.observe().expect("observed again").bytes, 0);
        assert_eq!(holding.observe_calls(), 2);
        assert_eq!(holding.empty_calls(), 1);

        let unsupported = MockRecycleBinBackend::unsupported();
        assert_eq!(unsupported.observe(), Err(RecycleBinError::Unsupported));
        assert_eq!(unsupported.empty(), Err(RecycleBinError::Unsupported));

        let refused = MockRecycleBinBackend::failing_observe(RecycleBinError::Failed(
            "the shell refused the query".to_string(),
        ));
        assert_eq!(
            refused.observe(),
            Err(RecycleBinError::Failed(
                "the shell refused the query".to_string()
            ))
        );

        // A refused empty attempt leaves what it covered in place, and the
        // observation — not the return code — is what a caller reports.
        let failed_empty = MockRecycleBinBackend::holding(2_048, 1)
            .with_empty_error(RecycleBinError::Failed("access denied".to_string()), 2_048);
        assert_eq!(
            failed_empty.empty(),
            Err(RecycleBinError::Failed("access denied".to_string()))
        );
        assert_eq!(
            failed_empty
                .observe()
                .expect("observed after a refusal")
                .bytes,
            2_048,
        );

        let cleared_despite_error = MockRecycleBinBackend::holding(2_048, 1).with_empty_error(
            RecycleBinError::Failed("the bin was already empty".to_string()),
            0,
        );
        assert!(cleared_despite_error.empty().is_err());
        assert_eq!(
            cleared_despite_error
                .observe()
                .expect("observed after the error")
                .bytes,
            0,
            "the observation is the measurement, not the return code"
        );

        let partial = MockRecycleBinBackend::holding(4_096, 2).with_empty_remaining(1_024);
        partial.empty().expect("a partial empty still succeeds");
        assert_eq!(
            partial.observe().expect("observed after a partial").bytes,
            1_024
        );
    }

    /// The unsupported answer is a refusal, not an error string a caller could
    /// mistake for a retryable state.
    #[test]
    fn unsupported_states_that_nothing_can_retry_it() {
        let described = RecycleBinError::Unsupported.describe();
        assert!(described.contains("no adapter"), "{described}");
        assert_eq!(
            RecycleBinError::Failed("access denied".to_string()).describe(),
            "access denied"
        );
    }

    /// A host with no shell adapter reports `Unsupported` rather than a
    /// synthesized measurement: the port states what it cannot do instead of
    /// answering zero, which would be indistinguishable from an empty bin.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_native_backend_reports_unsupported_where_no_adapter_exists() {
        let backend = NativeRecycleBinBackend;
        assert_eq!(backend.observe(), Err(RecycleBinError::Unsupported));
        assert_eq!(backend.empty(), Err(RecycleBinError::Unsupported));
    }

    /// The Windows build reaches the shell interface. `Failed` is accepted
    /// here because a machine whose shell session cannot answer is still
    /// evidence that the adapter exists — only `Unsupported` would mean the
    /// build has none, and that is what this refuses.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_windows_backend_reaches_the_shell_interface() {
        let backend = NativeRecycleBinBackend;
        match backend.observe() {
            Ok(observation) => assert!(observation.item_count <= u64::from(u32::MAX)),
            Err(RecycleBinError::Failed(message)) => {
                assert!(!message.is_empty(), "a failure states what the shell said");
            }
            Err(RecycleBinError::Unsupported) => {
                panic!("the Windows build implements the Recycle Bin interface")
            }
        }
    }
}
