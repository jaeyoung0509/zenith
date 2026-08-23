use std::sync::{Arc, Mutex};

/// Serializes filesystem-heavy and mutating storage workflows across all windows.
#[derive(Clone, Default)]
pub struct StorageOperationGate {
    inner: Arc<Mutex<()>>,
}

impl StorageOperationGate {
    pub fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _guard = self.inner.lock().expect("storage operation gate poisoned");
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
}
