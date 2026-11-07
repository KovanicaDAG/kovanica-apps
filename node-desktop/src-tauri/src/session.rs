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
    /// Copy of the poll gate's view sequence at the time this snapshot was taken.
    pub monitor_seq: u64,
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
        let (mut monitor, monitor_seq) = {
            let runtime = self.monitor.lock().expect("monitor lock");
            (runtime.gate.last.clone(), runtime.gate.view_seq)
        };
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
            monitor_seq,
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
        assert!(dead.process.status_seq > 1);
    }

    #[test]
    fn sequences_advance_on_lifecycle_and_a_stale_finish_does_not() {
        let (_tmp, session) = running_session();
        let running_seq = session.snapshot().process.status_seq;
        assert!(
            running_seq >= 2,
            "start must publish a newer process sequence"
        );
        session.attach_checked(sample_setup());
        let now = Instant::now();
        let BeginPoll::Fetch { generation, .. } = session.begin_monitor_poll(now) else {
            panic!("expected a fetch");
        };
        let during = session.snapshot().monitor_seq;
        session.finish_monitor_err(generation, now, "down".into());
        let failed = session.snapshot();
        assert!(failed.monitor_seq > during);
        assert_eq!(failed.monitor.rpc, RpcReach::Unreachable);
        assert!(failed.monitor.last_success_age_ms.is_none());
        assert!(!failed.monitor.stale);
        assert_ne!(failed.monitor.sync.committed_height, Field::value("0"));

        let later = now + crate::monitor::next_wait(1);
        let BeginPoll::Fetch {
            generation: ok_gen, ..
        } = session.begin_monitor_poll(later)
        else {
            panic!("expected a fetch after backoff");
        };
        let mut view = MonitorView::unknown();
        view.rpc = RpcReach::Reachable;
        view.sync.committed_height = Field::value("4");
        session.finish_monitor_ok(ok_gen, later, view);
        let live = session.snapshot();
        assert!(live.monitor_seq > failed.monitor_seq);
        assert_eq!(live.monitor.last_success_age_ms, Some(0));
        assert!(!live.monitor.stale);

        session.stop_owned().unwrap();
        let dead = session.snapshot();
        assert!(dead.process.status_seq > running_seq);
        assert_eq!(dead.process.state, ProcessState::Exited);
        let dead_monitor = dead.monitor_seq;
        let mut stale = MonitorView::unknown();
        stale.rpc = RpcReach::Reachable;
        stale.sync.committed_height = Field::value("99");
        session.finish_monitor_ok(ok_gen, Instant::now(), stale);
        let after = session.snapshot();
        assert_eq!(after.monitor_seq, dead_monitor);
        assert_ne!(after.monitor.sync.committed_height, Field::value("99"));
        assert_ne!(after.monitor.sync.committed_height, Field::value("0"));
        assert_eq!(after.process.status_seq, dead.process.status_seq);
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

    #[test]
    fn recovery_session_restart_rejects_stale_monitor() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        fs::create_dir(&data).unwrap();
        let marker = data.join("KEEP");
        fs::write(&marker, b"do-not-delete").unwrap();
        let binary = hold_script(tmp.path());
        let bystander_path = tmp.path().join("bystander.sh");
        fs::write(
            &bystander_path,
            "#!/bin/sh\nwhile true; do sleep 0.2; done\n",
        )
        .unwrap();
        let mut mode = fs::metadata(&bystander_path).unwrap().permissions();
        mode.set_mode(0o755);
        fs::set_permissions(&bystander_path, mode).unwrap();
        let mut bystander = std::process::Command::new(&bystander_path).spawn().unwrap();
        let bystander_pid = bystander.id();
        struct KillPid(u32);
        impl Drop for KillPid {
            fn drop(&mut self) {
                if self.0 != 0 {
                    unsafe {
                        libc::kill(self.0 as i32, libc::SIGKILL);
                    }
                }
            }
        }
        let _bystander_guard = KillPid(bystander_pid);

        let session = Session::new(Duration::from_millis(400));
        struct StopGuard<'a>(&'a Session);
        impl Drop for StopGuard<'_> {
            fn drop(&mut self) {
                let _ = self.0.stop_owned();
            }
        }
        let _stop_guard = StopGuard(&session);

        session
            .supervisor
            .start(&binary, Path::new("/tmp/unused.toml"))
            .unwrap();
        let first_pid = session.supervisor.status().pid.expect("owned pid");
        assert_ne!(first_pid, bystander_pid);
        session.attach_checked(sample_setup());
        let now = Instant::now();
        let BeginPoll::Fetch {
            generation: first_gen,
            ..
        } = session.begin_monitor_poll(now)
        else {
            panic!("expected a fetch on the first session");
        };
        let mut live = MonitorView::unknown();
        live.rpc = RpcReach::Reachable;
        live.sync.committed_height = Field::value("4");
        live.sync.local_tip_round = Field::value("4");
        live.health.status = Field::value("ok");
        session.finish_monitor_ok(first_gen, now, live);
        assert_eq!(
            session.snapshot().monitor.sync.committed_height,
            Field::value("4")
        );

        assert_eq!(unsafe { libc::kill(first_pid as i32, libc::SIGKILL) }, 0);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if session.supervisor.status().state == ProcessState::Exited {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "unexpected child exit was not observed"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let crashed = session.snapshot();
        assert_eq!(crashed.process.state, ProcessState::Exited);
        assert_eq!(crashed.process.pid, None);
        assert_ne!(crashed.monitor.sync.committed_height, Field::value("4"));
        assert_ne!(crashed.monitor.sync.committed_height, Field::value("0"));
        assert_eq!(crashed.monitor.health.status.kind, FieldKind::Unavailable);
        assert!(crashed.monitor.detail.unwrap().contains("not live"));
        assert!(
            process_alive(bystander_pid),
            "unexpected exit path killed a bystander"
        );

        let mut stale = MonitorView::unknown();
        stale.rpc = RpcReach::Reachable;
        stale.sync.committed_height = Field::value("99");
        session.finish_monitor_ok(first_gen, Instant::now(), stale);
        assert_ne!(
            session.snapshot().monitor.sync.committed_height,
            Field::value("99")
        );

        session
            .supervisor
            .start(&binary, Path::new("/tmp/unused.toml"))
            .unwrap();
        let second_pid = session.supervisor.status().pid.expect("restarted pid");
        assert_ne!(second_pid, first_pid);
        assert_ne!(second_pid, bystander_pid);
        session.attach_checked(sample_setup());
        let restarted = session.snapshot();
        assert_eq!(restarted.process.state, ProcessState::Running);
        assert_ne!(
            restarted.monitor.sync.committed_height,
            Field::value("4"),
            "pre-restart height survived into the new session"
        );
        assert_ne!(restarted.monitor.sync.committed_height, Field::value("99"));
        let later = Instant::now();
        let BeginPoll::Fetch {
            generation: second_gen,
            ..
        } = session.begin_monitor_poll(later)
        else {
            panic!("expected a fetch after restart");
        };
        assert_ne!(second_gen, first_gen);
        let mut fresh = MonitorView::unknown();
        fresh.rpc = RpcReach::Reachable;
        fresh.sync.committed_height = Field::value("7");
        fresh.sync.local_tip_round = Field::value("8");
        fresh.health.execution_state = Field::value("running");
        session.finish_monitor_ok(second_gen, later, fresh);
        session.finish_monitor_ok(first_gen, Instant::now(), {
            let mut old = MonitorView::unknown();
            old.rpc = RpcReach::Reachable;
            old.sync.committed_height = Field::value("99");
            old
        });
        session.finish_monitor_err(first_gen, Instant::now(), "stale session".into());
        let shown = session.snapshot();
        assert_eq!(shown.monitor.sync.committed_height, Field::value("7"));
        assert_eq!(shown.monitor.sync.local_tip_round, Field::value("8"));
        assert_eq!(shown.monitor.rpc, RpcReach::Reachable);
        assert!(!shown.monitor.stale);
        assert_eq!(shown.monitor.last_success_age_ms, Some(0));
        assert_eq!(fs::read(&marker).unwrap(), b"do-not-delete");

        session.stop_owned().unwrap();
        let stopped = session.snapshot();
        assert_eq!(stopped.process.state, ProcessState::Exited);
        assert_ne!(stopped.monitor.sync.committed_height, Field::value("7"));
        assert_ne!(stopped.monitor.sync.committed_height, Field::value("0"));
        assert!(process_alive(bystander_pid), "stop killed a bystander");
        assert!(!process_alive(second_pid));
        assert_eq!(fs::read(&marker).unwrap(), b"do-not-delete");
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    fn process_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}
