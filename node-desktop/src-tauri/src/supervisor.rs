//! Owns one `kovanica-node` child. Stop and exit only signal that child.
//!
//! Arguments are passed as argv (`--config`, path), never through a shell.
//! On Linux the child is placed in its own session so the signal goes to the
//! process group created for it. Stop sends SIGTERM, waits the grace period,
//! then SIGKILL, then waits until the child is actually reaped. A failed
//! signal or a wait that times out leaves the child owned and the state
//! `Stopping`. It is not reported as `Exited`. Nothing is killed by name,
//! and a pid of 0 is refused.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::redact::redact_log_line;

pub const LOG_LINES: usize = 200;
pub const STOP_GRACE: Duration = Duration::from_secs(10);
pub const KILL_WAIT: Duration = Duration::from_secs(2);

/// The one child this process owns, for the C `atexit` hook.
///
/// A GTK/WebKit `exit` during window teardown does not run Rust destructors
/// and can happen before `CloseRequested`. The hook signals only this pid,
/// and only if `/proc` still shows the same start time.
static OWNED_PID: AtomicI32 = AtomicI32::new(0);
static OWNED_START_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum SignalMode {
    Real,
    #[cfg(test)]
    Fail(std::io::ErrorKind),
    #[cfg(test)]
    NoopOk,
}

#[derive(Clone, Copy)]
enum WaitMode {
    Real,
    #[cfg(test)]
    AlwaysRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Exited,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStatus {
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub last_error: Option<String>,
    pub logs: Vec<String>,
}

struct Inner {
    child: Option<Child>,
    state: ProcessState,
    pid: Option<u32>,
    owns_group: bool,
    exit_code: Option<i32>,
    last_error: Option<String>,
    generation: u64,
}

pub struct Supervisor {
    inner: Mutex<Inner>,
    logs: Arc<Mutex<VecDeque<String>>>,
    grace: Duration,
    kill_wait: Duration,
    signal_mode: Mutex<SignalMode>,
    wait_mode: Mutex<WaitMode>,
}

impl Supervisor {
    pub fn new(grace: Duration) -> Self {
        Self::with_kill_wait(grace, KILL_WAIT)
    }

    pub(crate) fn with_kill_wait(grace: Duration, kill_wait: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                child: None,
                state: ProcessState::Stopped,
                pid: None,
                owns_group: false,
                exit_code: None,
                last_error: None,
                generation: 1,
            }),
            logs: Arc::new(Mutex::new(VecDeque::new())),
            grace,
            kill_wait,
            signal_mode: Mutex::new(SignalMode::Real),
            wait_mode: Mutex::new(WaitMode::Real),
        }
    }

    pub fn grace(&self) -> Duration {
        self.grace
    }

    pub fn generation(&self) -> u64 {
        self.lock().generation
    }

    pub fn status(&self) -> ProcessStatus {
        self.reap();
        let inner = self.lock();
        ProcessStatus {
            state: inner.state,
            pid: inner.pid,
            exit_code: inner.exit_code,
            last_error: inner.last_error.clone(),
            logs: self
                .logs
                .lock()
                .expect("log lock")
                .iter()
                .cloned()
                .collect(),
        }
    }

    /// Spawn `binary --config config`. The caller has already checked policy.
    pub fn start(&self, binary: &Path, config: &Path) -> Result<(), String> {
        {
            let inner = self.lock();
            if matches!(
                inner.state,
                ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
            ) {
                return Err(
                    "this app already has a node process. Stop it before starting another.".into(),
                );
            }
        }

        let mut command = Command::new(binary);
        command
            .arg("--config")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Safety: setsid runs in the forked child before exec, not in this process.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|err| {
            let mut inner = self.lock();
            inner.state = ProcessState::Stopped;
            inner.last_error = Some(format!("could not start {}: {err}", binary.display()));
            inner.last_error.clone().unwrap()
        })?;
        let pid = child.id();
        if pid == 0 {
            let _ = child.kill();
            let _ = child.wait();
            return Err("refusing a child pid of 0".into());
        }
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        {
            let mut inner = self.lock();
            inner.child = Some(child);
            inner.state = ProcessState::Running;
            inner.pid = Some(pid);
            inner.owns_group = true;
            inner.exit_code = None;
            inner.last_error = None;
            inner.generation = inner.generation.wrapping_add(1);
        }
        publish_owned(pid);
        {
            let mut logs = self.logs.lock().expect("log lock");
            logs.clear();
            push_log(
                &mut logs,
                format!("started pid {pid} with --config {}", config.display()),
            );
        }
        if let Some(stdout) = stdout {
            spawn_reader(Arc::clone(&self.logs), stdout);
        }
        if let Some(stderr) = stderr {
            spawn_reader(Arc::clone(&self.logs), stderr);
        }
        Ok(())
    }

    /// Stop only the child this supervisor spawned.
    ///
    /// `Ok` means the child was reaped, or there was no owned child.
    /// `Err` means the child may still be alive. The pid and handle stay, the
    /// state stays `Stopping`, and another `stop` retries the same pid.
    pub fn stop(&self) -> Result<(), String> {
        let pid = {
            let mut inner = self.lock();
            match inner.state {
                ProcessState::Starting | ProcessState::Running | ProcessState::Stopping => {}
                ProcessState::Stopped | ProcessState::Exited => return Ok(()),
            }
            let Some(pid) = inner.pid else {
                inner.last_error = Some(
                    "stopping without a pid. Ownership was not cleared and it was not reported as exited."
                        .into(),
                );
                return Err(inner.last_error.clone().unwrap());
            };
            if !inner.owns_group || pid == 0 || inner.child.is_none() {
                inner.state = ProcessState::Stopping;
                inner.last_error = Some(
                    "refusing to signal a process this app does not own. It was not reported as exited."
                        .into(),
                );
                return Err(inner.last_error.clone().unwrap());
            }
            inner.state = ProcessState::Stopping;
            pid
        };

        if self.confirmed_exit() {
            return Ok(());
        }

        let term_error = match self.dispatch_signal(pid, libc::SIGTERM) {
            Ok(()) => None,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if self.confirmed_exit() {
                    return Ok(());
                }
                Some(err.to_string())
            }
            Err(err) => Some(err.to_string()),
        };

        let deadline = Instant::now() + self.grace;
        loop {
            if self.confirmed_exit() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }

        match self.dispatch_signal(pid, libc::SIGKILL) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if self.confirmed_exit() {
                    return Ok(());
                }
                return self.unconfirmed(
                    pid,
                    format!("SIGKILL: {err}; earlier SIGTERM: {term_error:?}"),
                );
            }
            Err(err) => {
                return self.unconfirmed(pid, format!("SIGKILL failed: {err}"));
            }
        }

        let kill_deadline = Instant::now() + self.kill_wait;
        loop {
            if self.confirmed_exit() {
                return Ok(());
            }
            if Instant::now() >= kill_deadline {
                return self.unconfirmed(
                    pid,
                    format!(
                        "still running after SIGTERM ({} ms) and SIGKILL ({} ms)",
                        self.grace.as_millis(),
                        self.kill_wait.as_millis()
                    ),
                );
            }
            thread::sleep(Duration::from_millis(30));
        }
    }

    fn unconfirmed(&self, pid: u32, detail: String) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.pid != Some(pid) {
            return Err(
                "stop lost the owned pid and will not guess another process. It was not reported as exited."
                    .into(),
            );
        }
        inner.state = ProcessState::Stopping;
        inner.last_error = Some(format!(
            "owned pid {pid} was not confirmed exited ({detail}). Ownership kept; retry Stop."
        ));
        Err(inner.last_error.clone().unwrap())
    }

    fn confirmed_exit(&self) -> bool {
        let mode = *self.wait_mode.lock().expect("wait mode");
        match mode {
            WaitMode::Real => {}
            #[cfg(test)]
            WaitMode::AlwaysRunning => return false,
        }
        self.reap();
        self.finished()
    }

    fn dispatch_signal(&self, pid: u32, signal: i32) -> std::io::Result<()> {
        match *self.signal_mode.lock().expect("signal mode") {
            SignalMode::Real => signal_group(pid, signal),
            #[cfg(test)]
            SignalMode::NoopOk => Ok(()),
            #[cfg(test)]
            SignalMode::Fail(kind) => Err(std::io::Error::new(kind, "injected signal failure")),
        }
    }

    fn finished(&self) -> bool {
        matches!(
            self.lock().state,
            ProcessState::Exited | ProcessState::Stopped
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("supervisor lock")
    }

    fn reap(&self) {
        let mut inner = self.lock();
        let Some(child) = inner.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code();
                inner.exit_code = code;
                inner.pid = None;
                inner.owns_group = false;
                inner.last_error = None;
                inner.child = None;
                inner.state = ProcessState::Exited;
                clear_owned_publication();
                drop(inner);
                let mut logs = self.logs.lock().expect("log lock");
                push_log(
                    &mut logs,
                    format!(
                        "exited code {}",
                        code.map(|value| value.to_string())
                            .unwrap_or_else(|| "signal".into())
                    ),
                );
            }
            Ok(None) => {}
            Err(err) => {
                inner.last_error = Some(format!(
                    "could not wait for the child: {err}. Ownership kept; it was not reported as exited."
                ));
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_fail_signals(&self, kind: std::io::ErrorKind) {
        *self.signal_mode.lock().expect("signal mode") = SignalMode::Fail(kind);
    }

    #[cfg(test)]
    pub(crate) fn test_noop_signals(&self) {
        *self.signal_mode.lock().expect("signal mode") = SignalMode::NoopOk;
    }

    #[cfg(test)]
    pub(crate) fn test_wait_always_running(&self) {
        *self.wait_mode.lock().expect("wait mode") = WaitMode::AlwaysRunning;
    }

    #[cfg(test)]
    pub(crate) fn test_clear_hooks(&self) {
        *self.signal_mode.lock().expect("signal mode") = SignalMode::Real;
        *self.wait_mode.lock().expect("wait mode") = WaitMode::Real;
    }
}

fn push_log(logs: &mut VecDeque<String>, line: String) {
    if let Some(safe) = redact_log_line(&line) {
        logs.push_back(safe);
        while logs.len() > LOG_LINES {
            logs.pop_front();
        }
    }
}

fn spawn_reader<R: Read + Send + 'static>(logs: Arc<Mutex<VecDeque<String>>>, pipe: R) {
    thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Ok(mut logs) = logs.lock() {
                push_log(&mut logs, line);
            }
        }
    });
}

/// Register the exit hook as late as possible.
///
/// `atexit` runs last-registered first. GTK registers its own handlers during
/// startup; if those call `_exit`, an earlier hook never runs. `main` calls
/// this once the window exists.
pub fn arm_exit_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let rc = libc::atexit(stop_owned_on_process_exit);
        if rc != 0 {
            eprintln!("node exit hook was not installed ({rc})");
        }
    });
}

fn publish_owned(pid: u32) {
    if pid == 0 || pid > i32::MAX as u32 {
        clear_owned_publication();
        return;
    }
    match process_start_ticks(pid as i32) {
        Some(ticks) => {
            OWNED_START_TICKS.store(ticks, Ordering::SeqCst);
            OWNED_PID.store(pid as i32, Ordering::SeqCst);
        }
        None => clear_owned_publication(),
    }
}

fn clear_owned_publication() {
    OWNED_PID.store(0, Ordering::SeqCst);
    OWNED_START_TICKS.store(0, Ordering::SeqCst);
}

/// `starttime` from `/proc/pid/stat` (field 22). `None` if that pid is gone.
fn process_start_ticks(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rsplit_once(')')?.1;
    // Tokens after `comm` begin at field 3 (state). Field 22 is starttime.
    rest.split_whitespace().nth(19)?.parse().ok()
}

extern "C" fn stop_owned_on_process_exit() {
    let pid = OWNED_PID.swap(0, Ordering::SeqCst);
    let started = OWNED_START_TICKS.swap(0, Ordering::SeqCst);
    if pid <= 0 || started == 0 {
        return;
    }
    let msg = format!("node exit hook: owned pid {pid}\n");
    unsafe {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
    signal_if_same_process(pid, started);
}

/// SIGTERM, then SIGKILL, only if `pid` is still the process that started at `started`.
fn signal_if_same_process(pid: i32, started: u64) {
    if process_start_ticks(pid) != Some(started) {
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
        let wait = libc::timespec {
            tv_sec: 0,
            tv_nsec: 200_000_000,
        };
        libc::nanosleep(&wait, std::ptr::null_mut());
    }
    if process_start_ticks(pid) == Some(started) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
fn ignores_sigterm(pid: i32) -> bool {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("SigIgn:") {
            let Ok(mask) = u64::from_str_radix(rest.trim(), 16) else {
                return false;
            };
            return mask & (1u64 << (libc::SIGTERM - 1)) != 0;
        }
    }
    false
}

fn signal_group(pid: u32, signal: i32) -> std::io::Result<()> {
    if pid == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pid 0 is not a child",
        ));
    }
    // Safety: pid is the child we spawned and placed in its own session.
    let rc = unsafe { libc::kill(-(pid as i32), signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Command;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn process_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    struct KillPid(u32);
    impl Drop for KillPid {
        fn drop(&mut self) {
            if self.0 == 0 {
                return;
            }
            unsafe {
                libc::kill(-(self.0 as i32), libc::SIGKILL);
                libc::kill(self.0 as i32, libc::SIGKILL);
            }
        }
    }

    #[test]
    fn spawn_failure_does_not_look_running() {
        let supervisor = Supervisor::new(Duration::from_millis(200));
        let err = supervisor
            .start(
                Path::new("/tmp/kovanica-node-does-not-exist-m1"),
                Path::new("/tmp/no.toml"),
            )
            .unwrap_err();
        assert!(err.contains("could not start"), "{err}");
        let status = supervisor.status();
        assert_eq!(status.state, ProcessState::Stopped);
        assert!(status.pid.is_none());
    }

    #[test]
    fn duplicate_start_and_stop_do_not_touch_an_unrelated_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.sh",
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let other = script(
            tmp.path(),
            "bystander.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        );
        let mut bystander = Command::new(&other).spawn().unwrap();
        let bystander_pid = bystander.id();

        let supervisor = Supervisor::new(Duration::from_millis(400));
        supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap();
        let owned = supervisor.status().pid.expect("owned pid");
        let _owned_guard = KillPid(owned);
        assert_ne!(owned, bystander_pid);
        let err = supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap_err();
        assert!(err.contains("already"), "{err}");
        assert!(process_alive(bystander_pid));
        supervisor.stop().unwrap();
        let status = supervisor.status();
        assert_eq!(status.state, ProcessState::Exited);
        assert!(!process_alive(owned));
        assert!(process_alive(bystander_pid), "stop killed an unrelated pid");
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn child_exit_is_recorded_and_sigkill_fallback_is_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let quick = script(tmp.path(), "exit.sh", "#!/bin/sh\nexit 7\n");
        let supervisor = Supervisor::new(Duration::from_millis(200));
        supervisor
            .start(&quick, Path::new("/tmp/unused.toml"))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = supervisor.status();
            if status.state == ProcessState::Exited {
                assert_eq!(status.exit_code, Some(7));
                break;
            }
            assert!(Instant::now() < deadline, "child exit was not observed");
            thread::sleep(Duration::from_millis(20));
        }

        let stubborn = script(
            tmp.path(),
            "stubborn.sh",
            "#!/bin/sh\ntrap '' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let supervisor = Supervisor::new(Duration::from_millis(200));
        supervisor
            .start(&stubborn, Path::new("/tmp/unused.toml"))
            .unwrap();
        let pid = supervisor.status().pid.unwrap();
        let _guard = KillPid(pid);
        let started = Instant::now();
        supervisor.stop().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!process_alive(pid));
        assert_eq!(supervisor.status().state, ProcessState::Exited);
    }

    #[test]
    fn argv_preserves_spaces_and_logs_are_redacted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a b");
        fs::create_dir(&dir).unwrap();
        let marker = dir.join("argv.txt");
        let binary = script(
            &dir,
            "node bin.sh",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" \"$2\" > \"{}\"\necho passphrase=nope\nexit 0\n",
                marker.display()
            ),
        );
        let config = dir.join("my config.toml");
        fs::write(&config, "not read by the script\n").unwrap();
        let supervisor = Supervisor::new(Duration::from_millis(200));
        supervisor.start(&binary, &config).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = supervisor.status();
            if marker.exists()
                && status.state == ProcessState::Exited
                && status.logs.iter().any(|line| line.contains("[redacted]"))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "script did not record argv and a redacted log"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let recorded = fs::read_to_string(&marker).unwrap();
        assert!(recorded.contains("--config"), "{recorded}");
        assert!(recorded.contains("my config.toml"), "{recorded}");
        let logs = supervisor.status().logs.join("\n");
        assert!(logs.contains("[redacted]"), "{logs}");
        assert!(!logs.contains("passphrase=nope"), "{logs}");
    }

    #[test]
    fn data_directory_marker_survives_restart_of_the_owned_process() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("chain data");
        fs::create_dir(&data).unwrap();
        let marker = data.join("KEEP");
        fs::write(&marker, b"do-not-delete").unwrap();
        let before = fs::read(&marker).unwrap();
        let binary = script(
            tmp.path(),
            "node.sh",
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let supervisor = Supervisor::new(Duration::from_millis(300));
        supervisor
            .start(&binary, Path::new("/tmp/unused.toml"))
            .unwrap();
        supervisor.stop().unwrap();
        supervisor
            .start(&binary, Path::new("/tmp/unused.toml"))
            .unwrap();
        supervisor.stop().unwrap();
        assert_eq!(fs::read(&marker).unwrap(), before);
    }

    #[test]
    fn signal_failure_keeps_ownership_and_spares_a_bystander() {
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.sh",
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let other = script(
            tmp.path(),
            "bystander.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        );
        let mut bystander = Command::new(&other).spawn().unwrap();
        let bystander_pid = bystander.id();
        let supervisor =
            Supervisor::with_kill_wait(Duration::from_millis(40), Duration::from_millis(40));
        supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap();
        let owned = supervisor.status().pid.expect("owned pid");
        let _owned_guard = KillPid(owned);
        assert_ne!(owned, bystander_pid);
        supervisor.test_fail_signals(std::io::ErrorKind::PermissionDenied);
        let started = Instant::now();
        let err = supervisor.stop().unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2), "stop hung");
        assert!(err.contains("not confirmed exited"), "{err}");
        assert!(err.contains("Ownership kept"), "{err}");
        let status = supervisor.status();
        assert_eq!(status.state, ProcessState::Stopping);
        assert_eq!(status.pid, Some(owned));
        assert_ne!(status.state, ProcessState::Exited);
        assert!(process_alive(owned));
        assert!(process_alive(bystander_pid));
        let again = supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap_err();
        assert!(again.contains("already"), "{again}");
        supervisor.test_clear_hooks();
        supervisor.stop().unwrap();
        assert_eq!(supervisor.status().state, ProcessState::Exited);
        assert!(!process_alive(owned));
        assert!(
            process_alive(bystander_pid),
            "retry stop killed an unrelated pid"
        );
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn kill_wait_timeout_keeps_the_child_and_spares_a_bystander() {
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        );
        let other = script(
            tmp.path(),
            "bystander.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        );
        let mut bystander = Command::new(&other).spawn().unwrap();
        let bystander_pid = bystander.id();
        let supervisor =
            Supervisor::with_kill_wait(Duration::from_millis(40), Duration::from_millis(40));
        supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap();
        let owned = supervisor.status().pid.expect("owned pid");
        let _owned_guard = KillPid(owned);
        supervisor.test_noop_signals();
        supervisor.test_wait_always_running();
        let started = Instant::now();
        let err = supervisor.stop().unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "stop hung: {err}"
        );
        assert!(err.contains("not confirmed exited"), "{err}");
        assert!(err.contains("still running"), "{err}");
        let status = supervisor.status();
        assert_eq!(status.state, ProcessState::Stopping);
        assert_eq!(status.pid, Some(owned));
        assert!(status.last_error.is_some());
        assert!(process_alive(owned));
        assert!(process_alive(bystander_pid));
        supervisor.test_clear_hooks();
        supervisor.stop().unwrap();
        assert!(!process_alive(owned));
        assert_eq!(supervisor.status().state, ProcessState::Exited);
        assert!(
            process_alive(bystander_pid),
            "cleanup killed an unrelated pid"
        );
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn exit_hook_signals_only_the_matching_process_group() {
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.py",
            "#!/usr/bin/env python3\nimport signal, time\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nwhile True:\n    time.sleep(0.05)\n",
        );
        let other = script(
            tmp.path(),
            "bystander.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        );
        let mut owned = {
            let mut command = Command::new(&hold);
            unsafe {
                use std::os::unix::process::CommandExt;
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            command.spawn().unwrap()
        };
        let owned_pid = owned.id() as i32;
        let _owned_guard = KillPid(owned.id());
        let mut bystander = Command::new(&other).spawn().unwrap();
        let bystander_pid = bystander.id();
        let _bystander_guard = KillPid(bystander_pid);
        let ready = Instant::now() + Duration::from_secs(1);
        loop {
            if ignores_sigterm(owned_pid) {
                break;
            }
            assert!(Instant::now() < ready, "child never ignored SIGTERM");
            thread::sleep(Duration::from_millis(20));
        }
        let started = process_start_ticks(owned_pid).expect("start time");
        signal_if_same_process(owned_pid, started.wrapping_add(1));
        assert!(
            process_alive(owned.id()),
            "mismatched start time was signaled"
        );
        assert!(process_alive(bystander_pid));
        let began = Instant::now();
        signal_if_same_process(owned_pid, started);
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "exit hook was not bounded"
        );
        let status = owned.wait().expect("reap owned child");
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGKILL)
        );
        assert!(process_alive(bystander_pid), "exit hook killed a bystander");
        let _ = bystander.kill();
        let _ = bystander.wait();
    }
}
