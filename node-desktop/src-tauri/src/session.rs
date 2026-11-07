//! Shared setup, stop, and monitor orchestration used by the Tauri commands.
//!
//! `snapshot` locks the monitor mutex. Callers must drop that guard first.
//! A std mutex is not re-entrant, so taking it twice on one thread deadlocks.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::monitor::{HealthView, MonitorView, PollGate, SyncView};
use crate::policy::{CheckedSetup, REVIEWED_NODE_SOURCE};
use crate::supervisor::{ProcessState, ProcessStatus, Supervisor};

struct MonitorRuntime {
    gate: PollGate,
    rpc_http: Option<String>,
}

pub struct Session {
    pub supervisor: Supervisor,
    monitor: Mutex<MonitorRuntime>,
    setup: Mutex<Option<CheckedSetup>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub process: ProcessStatus,
    pub monitor: MonitorView,
    pub setup: Option<CheckedSetup>,
    pub reviewed_node_source: &'static str,
    pub stop_grace_secs: u64,
    pub exit_stops_owned_child: bool,
}

#[derive(Debug)]
pub enum BeginPoll {
    Idle(Box<Snapshot>),
    Fetch { generation: u64, rpc_http: String },
}

/// What the window-close and process-exit hooks must do with `stop_owned`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowClose {
    AllowExit,
    StayOpen { error: String },
}

pub fn window_close_decision(stop_result: Result<(), String>) -> WindowClose {
    match stop_result {
        Ok(()) => WindowClose::AllowExit,
        Err(error) => WindowClose::StayOpen { error },
    }
}

impl Session {
    pub fn new(grace: Duration) -> Self {
        Self {
            supervisor: Supervisor::new(grace),
            monitor: Mutex::new(MonitorRuntime {
                gate: PollGate::new(Instant::now()),
                rpc_http: None,
            }),
            setup: Mutex::new(None),
        }
    }

    pub fn store_setup(&self, checked: CheckedSetup) {
        *self.setup.lock().expect("setup lock") = Some(checked);
    }

    pub fn attach_checked(&self, checked: CheckedSetup) {
        {
            let mut monitor = self.monitor.lock().expect("monitor lock");
            monitor.gate.bump_generation(Instant::now());
            monitor.rpc_http = Some(checked.rpc_http.clone());
        }
        self.store_setup(checked);
    }

    pub fn set_paused(&self, paused: bool) {
        self.monitor.lock().expect("monitor lock").gate.paused = paused;
    }

    pub fn snapshot(&self) -> Snapshot {
        let process = self.supervisor.status();
        let mut monitor = self.monitor.lock().expect("monitor lock").gate.last.clone();
        // A reaped child must not keep the last healthy or not-catching-up read.
        if process.state == ProcessState::Exited {
            monitor.rpc = crate::monitor::RpcReach::Unreachable;
            monitor.health = HealthView::unavailable();
            monitor.sync = SyncView::unavailable();
            monitor.stale = false;
            monitor.last_success_age_ms = None;
            monitor.detail = Some(match process.exit_code {
                Some(code) => {
                    format!("owned process exited with code {code}; monitor is not live")
                }
                None => "owned process exited; monitor is not live".into(),
            });
        }
        Snapshot {
            process,
            monitor,
            setup: self.setup.lock().expect("setup lock").clone(),
            reviewed_node_source: REVIEWED_NODE_SOURCE,
            stop_grace_secs: self.supervisor.grace().as_secs(),
            exit_stops_owned_child: true,
        }
    }

    /// Same stop the Stop button and the window-close hook use.
    ///
    /// The loopback endpoint is dropped only after the child is confirmed
    /// reaped. A failed stop keeps it, so a retry still refers to that child.
    pub fn stop_owned(&self) -> Result<(), String> {
        let result = self.supervisor.stop();
        if result.is_ok() {
            let mut monitor = self.monitor.lock().expect("monitor lock");
            monitor.gate.bump_generation(Instant::now());
            monitor.rpc_http = None;
        }
        result
    }

    pub fn close_window(&self) -> WindowClose {
        window_close_decision(self.stop_owned())
    }

    /// Decide whether a monitor read may start. Does not call the network.
    ///
    /// The monitor lock is released before `snapshot`, including when the poll
    /// is paused, backing off, already in flight, or has no endpoint.
    pub fn begin_monitor_poll(&self, now: Instant) -> BeginPoll {
        if self.supervisor.status().state != ProcessState::Running {
            return BeginPoll::Idle(Box::new(self.snapshot()));
        }
        let mut monitor = self.monitor.lock().expect("monitor lock");
        let Some(generation) = monitor.gate.begin(now) else {
            drop(monitor);
            return BeginPoll::Idle(Box::new(self.snapshot()));
        };
        let Some(rpc_http) = monitor.rpc_http.clone() else {
            monitor
                .gate
                .finish_err(generation, now, "no loopback endpoint".into());
            drop(monitor);
            return BeginPoll::Idle(Box::new(self.snapshot()));
        };
        drop(monitor);
        BeginPoll::Fetch {
            generation,
            rpc_http,
        }
    }

    pub fn finish_monitor_ok(&self, generation: u64, now: Instant, view: MonitorView) {
        let mut monitor = self.monitor.lock().expect("monitor lock");
        monitor.gate.finish_ok(generation, now, view);
    }

    pub fn finish_monitor_err(&self, generation: u64, now: Instant, detail: String) {
        let mut monitor = self.monitor.lock().expect("monitor lock");
        monitor.gate.finish_err(generation, now, detail);
    }

    #[cfg(test)]
    fn endpoint(&self) -> Option<String> {
        self.monitor.lock().expect("monitor lock").rpc_http.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{Field, FieldKind, RpcReach};
    use crate::supervisor::STOP_GRACE;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;

    fn must_finish<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(work());
        });
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(value) => value,
            Err(_) => panic!("orchestration deadlocked or exceeded 2s"),
        }
    }

    fn hold_script(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("hold.sh");
        fs::write(
            &path,
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn running_session() -> (tempfile::TempDir, Session) {
        let tmp = tempfile::tempdir().unwrap();
        let binary = hold_script(tmp.path());
        let session = Session::new(Duration::from_millis(200));
        session
            .supervisor
            .start(&binary, Path::new("/tmp/unused.toml"))
            .unwrap();
        (tmp, session)
    }

    #[test]
    fn paused_in_flight_backoff_and_missing_endpoint_do_not_deadlock() {
        let (_tmp, session) = running_session();
        let session = std::sync::Arc::new(session);

        session.set_paused(true);
        let paused = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.begin_monitor_poll(Instant::now()))
        };
        assert!(
            matches!(paused, BeginPoll::Idle(_)),
            "paused poll started a fetch"
        );
        session.set_paused(false);

        session.attach_checked(sample_setup());
        let now = Instant::now();
        let first = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.begin_monitor_poll(now))
        };
        let BeginPoll::Fetch {
            generation,
            rpc_http,
        } = first
        else {
            panic!("expected a fetch while running");
        };
        assert!(rpc_http.contains("127.0.0.1"));
        let second = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.begin_monitor_poll(now))
        };
        assert!(
            matches!(second, BeginPoll::Idle(_)),
            "in-flight poll overlapped"
        );

        session.finish_monitor_err(generation, now, "down".into());
        let backed_off = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.begin_monitor_poll(now))
        };
        match backed_off {
            BeginPoll::Idle(snapshot) => {
                assert_eq!(snapshot.monitor.rpc, RpcReach::Unreachable);
                assert_ne!(snapshot.monitor.sync.catching_up, Field::value("false"));
                assert_ne!(
                    snapshot.monitor.sync.committed_height.value.as_deref(),
                    Some("0")
                );
            }
            BeginPoll::Fetch { .. } => panic!("backoff did not hold"),
        }

        {
            let mut monitor = session.monitor.lock().expect("monitor lock");
            monitor.rpc_http = None;
            monitor.gate.bump_generation(Instant::now());
        }
        let missing = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.begin_monitor_poll(Instant::now()))
        };
        match missing {
            BeginPoll::Idle(snapshot) => {
                assert_eq!(
                    snapshot.monitor.detail.as_deref(),
                    Some("no loopback endpoint")
                );
                assert_eq!(snapshot.monitor.rpc, RpcReach::Unreachable);
                assert_eq!(
                    snapshot.monitor.health.status.kind,
                    crate::monitor::FieldKind::Unavailable
                );
            }
            BeginPoll::Fetch { .. } => panic!("missing endpoint started a fetch"),
        }

        let _ = session.stop_owned();
        assert_eq!(session.supervisor.status().state, ProcessState::Exited);
    }

    #[test]
    fn failed_stop_keeps_the_endpoint_and_window_open() {
        let (_tmp, session) = running_session();
        let session = std::sync::Arc::new(session);
        session.attach_checked(sample_setup());
        let endpoint = session.endpoint().expect("endpoint");
        session
            .supervisor
            .test_fail_signals(std::io::ErrorKind::PermissionDenied);
        let owned = session.supervisor.status().pid.expect("pid");
        let decision = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.close_window())
        };
        match decision {
            WindowClose::StayOpen { error } => {
                assert!(error.contains("not confirmed exited"), "{error}");
            }
            WindowClose::AllowExit => panic!("unconfirmed stop allowed the window to close"),
        }
        let status = session.snapshot().process;
        assert_eq!(status.state, ProcessState::Stopping);
        assert_ne!(status.state, ProcessState::Exited);
        assert_eq!(status.pid, Some(owned));
        assert_eq!(session.endpoint().as_deref(), Some(endpoint.as_str()));
        session.supervisor.test_clear_hooks();
        let closed = {
            let session = std::sync::Arc::clone(&session);
            must_finish(move || session.close_window())
        };
        assert_eq!(closed, WindowClose::AllowExit);
        assert!(session.endpoint().is_none());
        assert_eq!(session.supervisor.status().state, ProcessState::Exited);
        assert!(unsafe { libc::kill(owned as i32, 0) } != 0);
    }

    #[test]
    fn exited_child_does_not_keep_a_healthy_monitor() {
        let (_tmp, session) = running_session();
        session.attach_checked(sample_setup());
        let now = Instant::now();
        let BeginPoll::Fetch { generation, .. } = session.begin_monitor_poll(now) else {
            panic!("expected a fetch");
        };
        let mut view = MonitorView::unknown();
        view.rpc = RpcReach::Reachable;
        view.health.status = Field::value("ok");
        view.health.healthy = Field::value("true");
        view.sync.committed_height = Field::value("4");
        view.sync.catching_up = Field::value("false");
        session.finish_monitor_ok(generation, now, view);
        assert_eq!(session.snapshot().monitor.health.status, Field::value("ok"));
        session.stop_owned().unwrap();
        let dead = session.snapshot();
        assert_eq!(dead.process.state, ProcessState::Exited);
        assert_eq!(dead.monitor.health.status.kind, FieldKind::Unavailable);
        assert_ne!(dead.monitor.health.healthy, Field::value("true"));
        assert_ne!(dead.monitor.sync.catching_up, Field::value("false"));
        assert_ne!(dead.monitor.sync.committed_height, Field::value("0"));
        assert!(dead.monitor.detail.unwrap().contains("not live"));
    }

    fn sample_setup() -> CheckedSetup {
        CheckedSetup {
            binary: "/tmp/unused".into(),
            binary_sha256: "ab".into(),
            config_path: "/tmp/unused.toml".into(),
            data_dir: "/tmp/data".into(),
            chain_id: 1337,
            chain_label: "local (chain 1337)".into(),
            chain_id_from_file: true,
            rpc_addr: "127.0.0.1".into(),
            rpc_port: 8545,
            rpc_http: "http://127.0.0.1:8545".into(),
            listen_addr: "127.0.0.1:8000".into(),
            mining_enabled: false,
            run_validator: false,
        }
    }

    #[test]
    fn production_grace_constant_is_ten_seconds() {
        assert_eq!(STOP_GRACE, Duration::from_secs(10));
        assert_eq!(window_close_decision(Ok(())), WindowClose::AllowExit);
        match window_close_decision(Err("still owned".into())) {
            WindowClose::StayOpen { error } => assert!(error.contains("owned")),
            WindowClose::AllowExit => panic!("unconfirmed stop would close the window"),
        }
    }
}
