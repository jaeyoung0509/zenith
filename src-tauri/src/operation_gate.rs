use std::sync::{Arc, Mutex};

/// Serializes filesystem-heavy and mutating storage workflows across all windows.
#[derive(Clone, Default)]
pub struct StorageOperationGate {
    inner: Arc<Mutex<()>>,
}

impl StorageOperationGate {
    pub fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        // Lifecycle-owned serialization gate: recover from poisoning so one
        // panicking operation cannot permanently wedge all storage workflows.
        let _guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        operation()
    }
}

#[cfg(test)]
mod tests {
    use super::StorageOperationGate;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn generic_and_storage_work_share_one_critical_section() {
        let gate = StorageOperationGate::default();
        let generic_gate = gate.clone();
        let storage_gate = gate.clone();
        let (generic_entered_tx, generic_entered_rx) = mpsc::channel();
        let (release_generic_tx, release_generic_rx) = mpsc::channel();
        let (storage_attempting_tx, storage_attempting_rx) = mpsc::channel();
        let (storage_entered_tx, storage_entered_rx) = mpsc::channel();

        let generic_work = thread::spawn(move || {
            generic_gate.run(|| {
                generic_entered_tx.send(()).unwrap();
                release_generic_rx.recv().unwrap();
            });
        });
        generic_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("generic work did not enter the gate");

        let storage_work = thread::spawn(move || {
            storage_attempting_tx.send(()).unwrap();
            storage_gate.run(|| storage_entered_tx.send(()).unwrap());
        });
        storage_attempting_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("storage work did not attempt to enter the gate");

        assert!(
            storage_entered_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "storage work entered while generic work still held the gate"
        );

        release_generic_tx.send(()).unwrap();
        storage_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("storage work did not enter after the gate was released");

        generic_work.join().unwrap();
        storage_work.join().unwrap();
    }

    #[test]
    fn gate_remains_usable_after_operation_panics() {
        let gate = StorageOperationGate::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gate.run(|| panic!("intentional operation panic"))
        }));
        assert!(result.is_err());
        // Poison recovery: the next operation must still serialize and run.
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran_clone = ran.clone();
        gate.run(|| {
            ran_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    }
}
