use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use zenith_lib::dev_ports::{
    list_listeners, release_listener, DevelopmentPortStore, RawListenerRecord, RealDevPortSystem,
};
use zenith_lib::models::{ListenerExposure, ListenerProtocol, ReleaseMode, ReleaseOutcome};

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn test_controlled_integration_ephemeral_loopback_release() {
    let script = "import sys, socket, time; s = socket.socket(socket.AF_INET, socket.SOCK_STREAM); s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]; s.listen(1); print(f'PORT:{port}', flush=True); time.sleep(60)";
    let mut child = Command::new("python3")
        .args(["-c", script, "-m", "http.server"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn python helper process");

    let pid = child.id();
    let stdout = child.stdout.take().expect("stdout piped");
    let _guard = ChildGuard { child };

    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .expect("read line from helper");

    assert!(
        first_line.starts_with("PORT:"),
        "Expected PORT: prefix, got: {first_line}"
    );
    let port: u16 = first_line
        .trim()
        .trim_start_matches("PORT:")
        .parse()
        .expect("valid port number");

    assert!(port >= 1024, "Ephemeral port should be >= 1024");

    let store = Mutex::new(DevelopmentPortStore::default());
    let real_sys = RealDevPortSystem::default();

    struct TestIntegrationSystem {
        real: RealDevPortSystem,
        test_pid: u32,
        test_port: u16,
    }

    impl zenith_lib::dev_ports::DevPortSystem for TestIntegrationSystem {
        fn current_uid(&self) -> u32 {
            self.real.current_uid()
        }

        fn own_pid(&self) -> u32 {
            self.real.own_pid()
        }

        fn discover_listeners(&self) -> Result<Vec<RawListenerRecord>, String> {
            let mut status = 0;
            let res = unsafe { libc::waitpid(self.test_pid as i32, &mut status, libc::WNOHANG) };
            let is_alive = res == 0;
            if is_alive {
                Ok(vec![RawListenerRecord {
                    pid: self.test_pid,
                    command: "python3".to_string(),
                    uid: Some(self.real.current_uid()),
                    port: self.test_port,
                    bind_address: "127.0.0.1".to_string(),
                    exposure: ListenerExposure::Loopback,
                    protocol: ListenerProtocol::Tcp,
                }])
            } else {
                Ok(Vec::new())
            }
        }

        fn get_process_info(&self, pid: u32) -> Option<zenith_lib::dev_ports::ProcessSnapshot> {
            self.real.get_process_info(pid)
        }

        fn send_signal(&self, pid: u32, signal: i32) -> Result<(), String> {
            self.real.send_signal(pid, signal)
        }

        fn sleep(&self, duration: Duration) {
            self.real.sleep(duration);
        }

        fn now(&self) -> Instant {
            self.real.now()
        }
    }

    let test_sys = TestIntegrationSystem {
        real: real_sys,
        test_pid: pid,
        test_port: port,
    };

    let listeners = list_listeners(&store, &test_sys).expect("list_listeners failed");
    assert_eq!(listeners.len(), 1);
    let dev_listener = &listeners[0];
    assert_eq!(dev_listener.pid, pid);
    assert_eq!(dev_listener.port, port);
    assert_eq!(dev_listener.server_name, "Python http.server");
    assert!(dev_listener.can_release);

    let release_res = release_listener(&store, &test_sys, &dev_listener.id, ReleaseMode::Graceful)
        .expect("release_listener failed");

    assert_eq!(release_res.outcome, ReleaseOutcome::Released);
    assert_eq!(release_res.port, port);

    // Verify child process is no longer alive
    let is_still_alive = unsafe { libc::kill(pid as i32, 0) == 0 };
    assert!(!is_still_alive, "Child process should no longer be running");
}
