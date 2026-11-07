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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::redact::redact_log_line;

pub const LOG_LINES: usize = 200;
pub const STOP_GRACE: Duration = Duration::from_secs(10);
pub const KILL_WAIT: Duration = Duration::from_secs(2);

/// X11 `BadWindow` and `BadDrawable`. GDK's handler for an untrapped error
/// calls `_exit(1)` on this build, which does not run `atexit`.
pub const X_BAD_WINDOW: u8 = 3;
pub const X_BAD_DRAWABLE: u8 = 9;

static WINDOW_TEARDOWN: AtomicBool = AtomicBool::new(false);

/// Record that a window delete has started. Later `BadWindow` / `BadDrawable`
/// may be ignored. The flag is never cleared: close is one-way.
pub fn note_window_teardown() {
    WINDOW_TEARDOWN.store(true, Ordering::SeqCst);
}

pub fn window_teardown_started() -> bool {
    WINDOW_TEARDOWN.load(Ordering::SeqCst)
}

/// Ignore only BadWindow and BadDrawable, and only after delete has started.
/// Every other code is the toolkit's, including these two before delete.
pub fn ignore_x_error_during_teardown(error_code: u8, teardown: bool) -> bool {
    teardown && (error_code == X_BAD_WINDOW || error_code == X_BAD_DRAWABLE)
}

/// GDK event types that mean the toplevel is going away.
/// `GDK_DELETE` = 0, `GDK_DESTROY` = 1, `GDK_WINDOW_STATE` = 32.
/// Withdrawn is bit 0 and iconified is bit 1. On this WebKitGTK build,
/// `XGetGeometry` hits a dead drawable inside the iconified window-state
/// event, before `delete-event`. A focus-only change (bit 7) is not teardown.
pub fn gdk_event_starts_teardown(event_type: i32, new_window_state: i32) -> bool {
    const GDK_DELETE: i32 = 0;
    const GDK_DESTROY: i32 = 1;
    const GDK_WINDOW_STATE: i32 = 32;
    const WITHDRAWN_OR_ICONIFIED: i32 = 1 | 2;
    event_type == GDK_DELETE
        || event_type == GDK_DESTROY
        || (event_type == GDK_WINDOW_STATE && new_window_state & WITHDRAWN_OR_ICONIFIED != 0)
}

/// The one child this process owns, for the C `atexit` hook.
///
/// A GTK/WebKit `exit` during window teardown does not run Rust destructors
/// and can happen before `CloseRequested`. The hook signals only this pid,
/// and only if `/proc` still shows the same start time.
static OWNED_PID: AtomicI32 = AtomicI32::new(0);
static OWNED_START_TICKS: AtomicU64 = AtomicU64::new(0);
/// Test-only. When set, the exit hook's `kill` calls fail and touch no pid.
#[cfg(test)]
static EXIT_SIGNAL_FAILS: AtomicBool = AtomicBool::new(false);

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
    Some(proc_stat(pid)?.start_ticks)
}

struct ProcStat {
    state: u8,
    start_ticks: u64,
}

fn proc_stat(pid: i32) -> Option<ProcStat> {
    if pid <= 0 {
        return None;
    }
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rsplit_once(')')?.1;
    // Tokens after `comm` begin at field 3 (state). Field 22 is starttime.
    let mut fields = rest.split_whitespace();
    let state = *fields.next()?.as_bytes().first()?;
    let start_ticks = fields.nth(18)?.parse().ok()?;
    Some(ProcStat { state, start_ticks })
}

/// Same start time, and not a zombie. A zombie keeps its pid and start time
/// until `wait`, so start time alone would escalate SIGKILL after a clean exit.
fn owned_still_running(pid: i32, started: u64) -> bool {
    matches!(
        proc_stat(pid),
        Some(stat) if stat.start_ticks == started && stat.state != b'Z' && stat.state != b'X'
    )
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
    signal_owned_group(pid, started, STOP_GRACE);
}

/// SIGTERM, then SIGKILL only if that same process is still alive after `grace`.
///
/// `started` is `/proc/<pid>/stat` starttime. A mismatch means the pid was
/// reused or the child already exited, and nothing is signaled. `grace` is
/// `STOP_GRACE` on the process-exit path (10s), not a fixed 200 ms.
/// A failed signal does not select a different pid.
fn signal_owned_group(pid: i32, started: u64, grace: Duration) -> OwnedSignal {
    if pid <= 0 {
        return OwnedSignal::Refused;
    }
    if !owned_still_running(pid, started) {
        return if process_start_ticks(pid) == Some(started) {
            OwnedSignal::Exited
        } else {
            OwnedSignal::NotOwned
        };
    }
    match send_group_signal(pid, libc::SIGTERM) {
        GroupSignal::Gone => {
            if !owned_still_running(pid, started) {
                return if process_start_ticks(pid) == Some(started) {
                    OwnedSignal::Exited
                } else {
                    OwnedSignal::NotOwned
                };
            }
            // `kill` reported ESRCH but this start time is still running.
            // Do not switch pids. Wait the grace, then escalate only if it matches.
        }
        GroupSignal::Sent | GroupSignal::Failed => {}
    }
    let deadline = Instant::now() + grace;
    loop {
        if !owned_still_running(pid, started) {
            return OwnedSignal::Exited;
        }
        if Instant::now() >= deadline {
            break;
        }
        let slice = libc::timespec {
            tv_sec: 0,
            tv_nsec: 30_000_000,
        };
        unsafe {
            libc::nanosleep(&slice, std::ptr::null_mut());
        }
    }
    if !owned_still_running(pid, started) {
        return OwnedSignal::Exited;
    }
    match send_group_signal(pid, libc::SIGKILL) {
        GroupSignal::Sent => OwnedSignal::Killed,
        GroupSignal::Gone => {
            if owned_still_running(pid, started) {
                OwnedSignal::KillFailed
            } else {
                OwnedSignal::Exited
            }
        }
        GroupSignal::Failed => OwnedSignal::KillFailed,
    }
}

/// Result of signaling one process group. `Failed` is not "this pid is gone":
/// errno is read only after `kill` itself runs, so a test injection or a
/// stale errno cannot be mistaken for ESRCH.
enum GroupSignal {
    Sent,
    Gone,
    Failed,
}

fn send_group_signal(pid: i32, signal: i32) -> GroupSignal {
    if pid <= 0 {
        return GroupSignal::Failed;
    }
    #[cfg(test)]
    if EXIT_SIGNAL_FAILS.load(Ordering::SeqCst) {
        return GroupSignal::Failed;
    }
    let rc = unsafe { libc::kill(-pid, signal) };
    if rc == 0 {
        return GroupSignal::Sent;
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        GroupSignal::Gone
    } else {
        GroupSignal::Failed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedSignal {
    /// Pid was 0 or negative. Nothing was signaled.
    Refused,
    /// Start time did not match, or the pid was already gone.
    NotOwned,
    /// The process left before SIGKILL. It was not force-killed.
    Exited,
    /// SIGKILL was sent to the same process group after the grace period.
    Killed,
    /// SIGKILL failed. The pid was not replaced with another one.
    KillFailed,
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

#[cfg(test)]
fn catches_sigterm(pid: i32) -> bool {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("SigCgt:") {
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

    fn spawn_session(binary: &Path) -> Child {
        let mut command = Command::new(binary);
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
    }

    #[test]
    fn x_errors_are_ignored_only_for_teardown_drawable_codes() {
        assert!(!window_teardown_started());
        for code in [0u8, 1, 2, 3, 8, 9, 10, 11, 255] {
            assert!(
                !ignore_x_error_during_teardown(code, false),
                "code {code} was ignored before teardown"
            );
        }
        assert!(ignore_x_error_during_teardown(X_BAD_WINDOW, true));
        assert!(ignore_x_error_during_teardown(X_BAD_DRAWABLE, true));
        for code in [0u8, 1, 2, 8, 10, 11, 255] {
            assert!(
                !ignore_x_error_during_teardown(code, true),
                "code {code} was ignored during teardown"
            );
        }
        assert!(gdk_event_starts_teardown(0, 0));
        assert!(gdk_event_starts_teardown(1, 0));
        assert!(gdk_event_starts_teardown(32, 1));
        assert!(gdk_event_starts_teardown(32, 2));
        assert!(gdk_event_starts_teardown(32, (1 << 7) | 2));
        assert!(
            !gdk_event_starts_teardown(32, 1 << 7),
            "focus must not arm teardown"
        );
        assert!(!gdk_event_starts_teardown(32, 0));
        assert!(!gdk_event_starts_teardown(2, 1));
        assert!(!gdk_event_starts_teardown(12, 1));
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
    fn exit_hook_waits_out_grace_checks_ownership_and_reports_signal_failure() {
        assert_eq!(STOP_GRACE, Duration::from_secs(10));
        assert!(STOP_GRACE > Duration::from_millis(200));

        let tmp = tempfile::tempdir().unwrap();
        let slow = script(
            tmp.path(),
            "slow.py",
            "#!/usr/bin/env python3\nimport signal, sys, time\ndef on_term(_s, _f):\n    time.sleep(0.45)\n    sys.exit(0)\nsignal.signal(signal.SIGTERM, on_term)\nwhile True:\n    time.sleep(0.05)\n",
        );
        let stubborn = script(
            tmp.path(),
            "stubborn.py",
            "#!/usr/bin/env python3\nimport signal, time\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nwhile True:\n    time.sleep(0.05)\n",
        );
        let other = script(
            tmp.path(),
            "bystander.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        );
        let mut bystander = Command::new(&other).spawn().unwrap();
        let bystander_pid = bystander.id();
        let _bystander_guard = KillPid(bystander_pid);

        assert_eq!(
            signal_owned_group(0, 1, Duration::from_millis(50)),
            OwnedSignal::Refused
        );
        assert_eq!(
            signal_owned_group(bystander_pid as i32, 1, Duration::from_millis(50)),
            OwnedSignal::NotOwned
        );
        assert!(process_alive(bystander_pid));

        let mut slow_child = spawn_session(&slow);
        let slow_pid = slow_child.id();
        let _slow_guard = KillPid(slow_pid);
        let ready = Instant::now() + Duration::from_secs(2);
        loop {
            if catches_sigterm(slow_pid as i32) {
                break;
            }
            assert!(
                Instant::now() < ready,
                "slow child never installed a SIGTERM handler"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let slow_started = process_start_ticks(slow_pid as i32).expect("slow start");
        let began = Instant::now();
        let outcome =
            signal_owned_group(slow_pid as i32, slow_started, Duration::from_millis(1200));
        let elapsed = began.elapsed();
        assert_eq!(outcome, OwnedSignal::Exited, "slow child was force-killed");
        assert!(
            elapsed > Duration::from_millis(200),
            "exit hook did not wait past 200 ms: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1200),
            "exit hook waited for SIGKILL after the child had exited: {elapsed:?}"
        );
        let status = slow_child.wait().expect("reap slow child");
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            None,
            "child that exits after SIGTERM was SIGKILLed"
        );
        assert!(process_alive(bystander_pid));

        let mut stubborn_child = spawn_session(&stubborn);
        let stubborn_pid = stubborn_child.id() as i32;
        let _stubborn_guard = KillPid(stubborn_child.id());
        let ready = Instant::now() + Duration::from_secs(1);
        loop {
            if ignores_sigterm(stubborn_pid) {
                break;
            }
            assert!(Instant::now() < ready, "child never ignored SIGTERM");
            thread::sleep(Duration::from_millis(20));
        }
        let stubborn_started = process_start_ticks(stubborn_pid).expect("stubborn start");
        let grace = Duration::from_millis(400);
        let began = Instant::now();
        let outcome = signal_owned_group(stubborn_pid, stubborn_started, grace);
        let elapsed = began.elapsed();
        assert_eq!(outcome, OwnedSignal::Killed);
        assert!(
            elapsed >= grace,
            "SIGKILL ran before the grace period: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "escalation was not bounded"
        );
        let status = stubborn_child.wait().expect("reap stubborn child");
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGKILL)
        );
        assert!(
            process_alive(bystander_pid),
            "escalation killed a bystander"
        );

        let held = spawn_session(&stubborn);
        let held_pid = held.id() as i32;
        let _held_guard = KillPid(held.id());
        let held_started = process_start_ticks(held_pid).expect("held start");
        EXIT_SIGNAL_FAILS.store(true, Ordering::SeqCst);
        let began = Instant::now();
        let outcome = signal_owned_group(held_pid, held_started, Duration::from_millis(300));
        let elapsed = began.elapsed();
        EXIT_SIGNAL_FAILS.store(false, Ordering::SeqCst);
        assert_eq!(outcome, OwnedSignal::KillFailed);
        assert!(elapsed >= Duration::from_millis(300));
        assert!(elapsed < Duration::from_secs(2));
        assert!(
            process_alive(held.id()),
            "a failed signal dropped the child"
        );
        assert!(
            process_alive(bystander_pid),
            "a failed signal hit a bystander"
        );
        let _ = bystander.kill();
        let _ = bystander.wait();
    }
}
