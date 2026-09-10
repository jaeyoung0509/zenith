use std::sync::{Arc, RwLock};

/// Coordinates filesystem-heavy reads and mutating storage workflows across all windows.
/// Multiple reads may run concurrently, while mutations have exclusive access.
#[derive(Clone, Default)]
pub struct StorageOperationGate {
    inner: Arc<RwLock<()>>,
}

impl StorageOperationGate {
    /// Executes a read operation (concurrent with other reads, excluded by mutations).
    ///
    /// Recovers from lock poisoning so a panicking read or write operation cannot
    /// permanently wedge storage reads.
    pub fn run_read<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _guard = self
            .inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        operation()
    }

    /// Executes a mutating operation with exclusive access across all storage workflows.
    ///
    /// Recovers from lock poisoning so a panicking mutation cannot permanently wedge
    /// future storage operations.
    pub fn run_write<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _guard = self
            .inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        operation()
    }

    /// Backward-compatible alias for mutating/exclusive execution.
    pub fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        self.run_write(operation)
    }
}

#[cfg(test)]
mod tests {
    use super::StorageOperationGate;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
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
            generic_gate.run_write(|| {
                generic_entered_tx.send(()).unwrap();
                release_generic_rx.recv().unwrap();
            });
        });
        generic_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("generic work did not enter the gate");

        let storage_work = thread::spawn(move || {
            storage_attempting_tx.send(()).unwrap();
            storage_gate.run_write(|| storage_entered_tx.send(()).unwrap());
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
    fn reads_run_concurrently() {
        let gate = StorageOperationGate::default();
        let r1_gate = gate.clone();
        let r2_gate = gate.clone();
        let (r1_entered_tx, r1_entered_rx) = mpsc::channel();
        let (r2_entered_tx, r2_entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let t1 = thread::spawn(move || {
            r1_gate.run_read(|| {
                r1_entered_tx.send(()).unwrap();
                let _ = release_rx.recv();
            });
        });

        r1_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader 1 did not enter");

        let t2 = thread::spawn(move || {
            r2_gate.run_read(|| {
                r2_entered_tx.send(()).unwrap();
            });
        });

        // Reader 2 should enter even while Reader 1 is still holding the read lock.
        r2_entered_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("reader 2 was blocked by reader 1");

        let _ = release_tx.send(());
        t1.join().unwrap();
        t2.join().unwrap();
    }

    #[test]
    fn write_blocks_concurrent_reads() {
        let gate = StorageOperationGate::default();
        let write_gate = gate.clone();
        let read_gate = gate.clone();
        let (write_entered_tx, write_entered_rx) = mpsc::channel();
        let (release_write_tx, release_write_rx) = mpsc::channel();
        let (read_entered_tx, read_entered_rx) = mpsc::channel();

        let writer = thread::spawn(move || {
            write_gate.run_write(|| {
                write_entered_tx.send(()).unwrap();
                release_write_rx.recv().unwrap();
            });
        });

        write_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer did not enter");

        let reader = thread::spawn(move || {
            read_gate.run_read(|| {
                read_entered_tx.send(()).unwrap();
            });
        });

        assert!(
            read_entered_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "reader entered while writer held the gate"
        );

        release_write_tx.send(()).unwrap();
        read_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader did not enter after writer released");

        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn read_blocks_concurrent_writes() {
        let gate = StorageOperationGate::default();
        let read_gate = gate.clone();
        let write_gate = gate.clone();
        let (read_entered_tx, read_entered_rx) = mpsc::channel();
        let (release_read_tx, release_read_rx) = mpsc::channel();
        let (write_entered_tx, write_entered_rx) = mpsc::channel();

        let reader = thread::spawn(move || {
            read_gate.run_read(|| {
                read_entered_tx.send(()).unwrap();
                release_read_rx.recv().unwrap();
            });
        });

        read_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reader did not enter");

        let writer = thread::spawn(move || {
            write_gate.run_write(|| {
                write_entered_tx.send(()).unwrap();
            });
        });

        assert!(
            write_entered_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "writer entered while reader held the gate"
        );

        release_read_tx.send(()).unwrap();
        write_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("writer did not enter after reader released");

        reader.join().unwrap();
        writer.join().unwrap();
    }

    #[test]
    fn gate_remains_usable_after_write_operation_panics() {
        let gate = StorageOperationGate::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gate.run_write(|| panic!("intentional write panic"))
        }));
        assert!(result.is_err());

        // Poison recovery: both read and write must still operate.
        let ran_read = Arc::new(AtomicBool::new(false));
        let ran_read_clone = ran_read.clone();
        gate.run_read(|| {
            ran_read_clone.store(true, Ordering::SeqCst);
        });
        assert!(ran_read.load(Ordering::SeqCst));

        let ran_write = Arc::new(AtomicBool::new(false));
        let ran_write_clone = ran_write.clone();
        gate.run_write(|| {
            ran_write_clone.store(true, Ordering::SeqCst);
        });
        assert!(ran_write.load(Ordering::SeqCst));
    }

    #[test]
    fn gate_remains_usable_after_read_operation_panics() {
        let gate = StorageOperationGate::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gate.run_read(|| panic!("intentional read panic"))
        }));
        assert!(result.is_err());

        let ran_read = Arc::new(AtomicBool::new(false));
        let ran_read_clone = ran_read.clone();
        gate.run_read(|| {
            ran_read_clone.store(true, Ordering::SeqCst);
        });
        assert!(ran_read.load(Ordering::SeqCst));

        let ran_write = Arc::new(AtomicBool::new(false));
        let ran_write_clone = ran_write.clone();
        gate.run_write(|| {
            ran_write_clone.store(true, Ordering::SeqCst);
        });
        assert!(ran_write.load(Ordering::SeqCst));
    }
}
