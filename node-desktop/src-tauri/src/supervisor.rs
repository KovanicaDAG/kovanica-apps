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

/// XErrorEvent structure from X11/Xlib.h.
#[repr(C)]
#[allow(dead_code)]
pub struct XErrorEvent {
    pub type_: i32,
    pub display: *mut std::ffi::c_void,
    pub resourceid: usize,
    pub serial: usize,
    pub error_code: u8,
    pub request_code: u8,
    pub minor_code: u8,
}

pub type XErrorHandler = unsafe extern "C" fn(*mut std::ffi::c_void, *mut XErrorEvent) -> i32;

static WINDOW_TEARDOWN: AtomicBool = AtomicBool::new(false);
static TRACKED_WINDOW_XID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Record that window teardown has started.
pub fn note_window_teardown() {
    WINDOW_TEARDOWN.store(true, Ordering::SeqCst);
}

/// Disarm window teardown (e.g. when close is cancelled or unconfirmed).
pub fn disarm_window_teardown() {
    WINDOW_TEARDOWN.store(false, Ordering::SeqCst);
}

/// Return whether window teardown is actively in progress.
pub fn window_teardown_started() -> bool {
    WINDOW_TEARDOWN.load(Ordering::SeqCst)
}

/// Set the tracked toplevel window XID.
pub fn set_tracked_window_xid(xid: usize) {
    TRACKED_WINDOW_XID.store(xid, Ordering::SeqCst);
}

/// Return the tracked toplevel window XID.
pub fn tracked_window_xid() -> usize {
    TRACKED_WINDOW_XID.load(Ordering::SeqCst)
}

/// Ignore only BadWindow and BadDrawable, and only while teardown is active.
/// Every other code is the toolkit's, including these two before teardown.
///
/// This helper is not resource-aware. Production suppression is
/// [`should_suppress_x_error`], which also requires the tracked toplevel XID.
pub fn ignore_x_error_during_teardown(error_code: u8, teardown: bool) -> bool {
    teardown && (error_code == X_BAD_WINDOW || error_code == X_BAD_DRAWABLE)
}

/// `GdkEventType::GDK_DESTROY`.
pub const GDK_DESTROY: i32 = 1;
/// `GdkEventType::GDK_WINDOW_STATE`.
pub const GDK_WINDOW_STATE: i32 = 32;
/// `GdkWindowState::GDK_WINDOW_STATE_ICONIFIED` (`1 << 1`). Minimize and
/// restore both set this bit in `changed_mask`. It is not window destruction.
pub const GDK_WINDOW_STATE_ICONIFIED: i32 = 1 << 1;

/// Suppress only `BadWindow` / `BadDrawable` for the demonstrated tracked
/// toplevel resource, and only while teardown is already armed.
///
/// `resource_id` must equal `tracked_xid`, and `tracked_xid` must be non-zero.
/// A teardown flag alone does not name a resource. Iconification, restore, and
/// stale GDK event metadata are not inputs: those are not approved close.
pub fn should_suppress_x_error(
    error_code: u8,
    resource_id: usize,
    teardown: bool,
    tracked_xid: usize,
) -> bool {
    teardown
        && tracked_xid != 0
        && resource_id == tracked_xid
        && (error_code == X_BAD_WINDOW || error_code == X_BAD_DRAWABLE)
}

/// Check if a GDK event initiates teardown for a specific window.
///
/// Only `GDK_DESTROY` (1) on the monitored toplevel window represents actual
/// destruction. Window state events (32) such as minimize / iconify (bit 1),
/// withdraw (bit 0), focus (bit 7), or restore are normal window operations
/// and MUST NOT arm teardown. Auxiliary windows (where `is_tracked_window == false`)
/// must not arm teardown.
pub fn gdk_event_starts_teardown_for_window(
    event_type: i32,
    new_window_state: i32,
    is_tracked_window: bool,
) -> bool {
    let _ = new_window_state;
    is_tracked_window && event_type == GDK_DESTROY
}

/// Backward compatible helper: only `GDK_DESTROY` (1) represents actual destruction.
/// Window state changes (including minimize, restore, and withdraw) never start teardown.
pub fn gdk_event_starts_teardown(event_type: i32, new_window_state: i32) -> bool {
    let _ = new_window_state;
    event_type == GDK_DESTROY
}

/// `GdkEventType` of the event currently inside `gtk_main_do_event`.
/// `-1` means no dispatch is in progress. Cleared when that call returns so a
/// later X error is not attributed to a minimize or restore that already finished.
static CURRENT_GDK_EVENT: AtomicI32 = AtomicI32::new(-1);
static LAST_WINDOW_STATE_CHANGED: AtomicI32 = AtomicI32::new(0);
static LAST_WINDOW_STATE_NEW: AtomicI32 = AtomicI32::new(0);

/// Record the GDK event the production handler is dispatching.
///
/// Does not arm teardown. Window-state `changed_mask` bit 1 (iconified) is
/// stored only so a forwarded error can be logged. It is not a close.
pub fn note_gdk_dispatch(event_type: i32, changed_mask: i32, new_window_state: i32) {
    CURRENT_GDK_EVENT.store(event_type, Ordering::SeqCst);
    if event_type == GDK_WINDOW_STATE {
        LAST_WINDOW_STATE_CHANGED.store(changed_mask, Ordering::SeqCst);
        LAST_WINDOW_STATE_NEW.store(new_window_state, Ordering::SeqCst);
    }
}

/// Drop the in-dispatch GDK event. Call this after `gtk_main_do_event` returns.
pub fn clear_gdk_dispatch() {
    CURRENT_GDK_EVENT.store(-1, Ordering::SeqCst);
    LAST_WINDOW_STATE_CHANGED.store(0, Ordering::SeqCst);
    LAST_WINDOW_STATE_NEW.store(0, Ordering::SeqCst);
}

/// Event type currently inside GDK dispatch, or `-1` after it returns.
pub fn current_gdk_event() -> i32 {
    CURRENT_GDK_EVENT.load(Ordering::SeqCst)
}

/// `(changed_mask, new_window_state)` from the GDK event in progress.
/// Both are zero after [`clear_gdk_dispatch`].
pub fn gdk_window_state_snapshot() -> (i32, i32) {
    (
        LAST_WINDOW_STATE_CHANGED.load(Ordering::SeqCst),
        LAST_WINDOW_STATE_NEW.load(Ordering::SeqCst),
    )
}

/// Delete-event emission: the tracked widget, or any widget when none is recorded yet.
pub fn delete_event_targets_tracked(
    tracked_widget: *mut std::ffi::c_void,
    emitting_widget: *mut std::ffi::c_void,
) -> bool {
    tracked_widget.is_null() || emitting_widget == tracked_widget
}

fn write_stderr(msg: &str) {
    unsafe {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
}

/// Production `XSetErrorHandler` body.
///
/// Suppression uses [`should_suppress_x_error`] only. A tracked-window
/// `BadWindow`/`BadDrawable` that lines up with `GDK_WINDOW_STATE` and
/// iconification is not an approved close and is not swallowed here.
///
/// The previous handler is invoked as-is. If it returns, the owned child is
/// left running: a forwarded error is not assumed fatal. GTK 3.24's handler
/// (`gdk_x_error` → `_gdk_x11_display_error_event`) returns for a display it
/// does not own or for a serial covered by an error trap, and `_exit(1)`s for
/// an untrapped error on a GDK display. That `_exit` skips `atexit`, so the
/// child stop for process death is the parent-death reaper armed with the
/// owned pid, not a stop placed before this call.
///
/// # Safety
/// `event` must be a live `XErrorEvent` for the duration of the call, or null.
/// `previous`, when set, must be an X error handler.
pub unsafe fn dispatch_x_error(
    display: *mut std::ffi::c_void,
    event: *mut XErrorEvent,
    previous: Option<XErrorHandler>,
) -> i32 {
    if event.is_null() {
        write_stderr("node x error ignored null event\n");
        return 0;
    }
    let code = unsafe { (*event).error_code };
    let resource_id = unsafe { (*event).resourceid };
    let request = unsafe { (*event).request_code };
    let teardown = window_teardown_started();
    let tracked_xid = tracked_window_xid();
    if should_suppress_x_error(code, resource_id, teardown, tracked_xid) {
        write_stderr(&format!(
            "node x error {code} ignored during teardown resource={resource_id}\n"
        ));
        return 0;
    }
    if code == X_BAD_WINDOW || code == X_BAD_DRAWABLE {
        let gdk_event = current_gdk_event();
        let (changed, new_state) = gdk_window_state_snapshot();
        write_stderr(&format!(
            "node x error {code} passed to toolkit teardown={teardown} request={request} resource={resource_id} tracked={tracked_xid} gdk_event={gdk_event} window_changed={changed} window_new={new_state}\n"
        ));
    }
    if let Some(previous) = previous {
        return unsafe { previous(display, event) };
    }
    write_stderr(&format!("node x error {code} had no previous handler\n"));
    0
}

/// The one child this process owns, for the C `atexit` hook.
///
/// A GTK/WebKit `exit` during window teardown does not run Rust destructors
/// and can happen before `CloseRequested`. The hook signals only this pid,
/// and only if `/proc` still shows the same start time.
static OWNED_PID: AtomicI32 = AtomicI32::new(0);
static OWNED_START_TICKS: AtomicU64 = AtomicU64::new(0);
/// Set by the GUI process before it can own a child. Library tests leave this
/// false so `start` does not re-exec the test harness.
static PARENT_DEATH_REAPER_ENABLED: AtomicBool = AtomicBool::new(false);
static REAPER_CHILD: Mutex<Option<Child>> = Mutex::new(None);
/// Test-only replacement for `current_exe`. Production arming re-execs this
/// binary; tests point it at a script so the test harness is not re-execed.
#[cfg(test)]
static REAPER_EXE_OVERRIDE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
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
        if let Err(err) = publish_owned(pid) {
            let stop_result = self.stop();
            return match stop_result {
                Ok(()) => Err(format!("{err} The child was stopped.")),
                Err(stop_err) => Err(format!(
                    "{err} The child was not confirmed stopped: {stop_err}"
                )),
            };
        }
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
                disarm_parent_death_reaper();
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

fn publish_owned(pid: u32) -> Result<(), String> {
    if pid == 0 || pid > i32::MAX as u32 {
        clear_owned_publication();
        disarm_parent_death_reaper();
        return Err("refusing to publish a child pid of 0".into());
    }
    let Some(ticks) = process_start_ticks(pid as i32) else {
        clear_owned_publication();
        disarm_parent_death_reaper();
        return Err(
            "could not read the child start time, so it was not published to the reaper".into(),
        );
    };
    OWNED_START_TICKS.store(ticks, Ordering::SeqCst);
    OWNED_PID.store(pid as i32, Ordering::SeqCst);
    if let Err(err) = arm_parent_death_reaper(pid as i32, ticks) {
        clear_owned_publication();
        return Err(err);
    }
    Ok(())
}

fn clear_owned_publication() {
    OWNED_PID.store(0, Ordering::SeqCst);
    OWNED_START_TICKS.store(0, Ordering::SeqCst);
}

/// Arm the process-death stop used when GTK `_exit`s from an untrapped X error.
///
/// The GUI calls this once, before a child can be published. The reaper is a
/// separate process because `_exit` does not run `atexit` and does not return
/// to `dispatch_x_error`. It is not armed for an X error whose previous handler
/// returns.
pub fn enable_parent_death_reaper() {
    PARENT_DEATH_REAPER_ENABLED.store(true, Ordering::SeqCst);
}

/// Arguments after the program name. `Ok(None)` means this is not a reaper.
///
/// Order: parent pid, parent start ticks, node pid, node start ticks.
pub fn parse_reaper_args<I>(args: I) -> Result<Option<(i32, u64, i32, u64)>, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(None);
    };
    if first != "--parent-death-reaper" {
        return Ok(None);
    }
    let parent_pid = args
        .next()
        .ok_or("parent-death reaper is missing the parent pid")?
        .parse::<i32>()
        .map_err(|_| "parent-death reaper parent pid is not an integer")?;
    let parent_started = args
        .next()
        .ok_or("parent-death reaper is missing the parent start time")?
        .parse::<u64>()
        .map_err(|_| "parent-death reaper parent start time is not an integer")?;
    let node_pid = args
        .next()
        .ok_or("parent-death reaper is missing the node pid")?
        .parse::<i32>()
        .map_err(|_| "parent-death reaper node pid is not an integer")?;
    let node_started = args
        .next()
        .ok_or("parent-death reaper is missing the node start time")?
        .parse::<u64>()
        .map_err(|_| "parent-death reaper node start time is not an integer")?;
    if parent_pid <= 0
        || node_pid <= 0
        || parent_started == 0
        || node_started == 0
        || parent_pid == node_pid
    {
        return Err(
            "parent-death reaper requires a positive parent pid, a different positive node pid, and non-zero start times"
                .into(),
        );
    }
    Ok(Some((parent_pid, parent_started, node_pid, node_started)))
}

/// True only for the process this supervisor just spawned.
///
/// While the parent is still the process whose start time was published, the
/// node must be its direct child. If that parent has already exited, the node
/// must have been reparented to init (ppid 1). In both cases it must be a
/// process-group leader (`pgrp == pid`, which `setsid` establishes) and the
/// node's start time must be the published one, not earlier than the parent's.
/// A reused pid fails the start-time check. A bystander whose parent is some
/// other live process fails the ppid check. This is the process relationship
/// recorded at arm time, not a guess about window state.
fn adopt_reaper_target(
    parent_pid: i32,
    parent_started: u64,
    node_pid: i32,
    node_started: u64,
) -> bool {
    let Some(node) = proc_stat(node_pid) else {
        return false;
    };
    if node.start_ticks != node_started
        || node.pgrp != node_pid
        || node.start_ticks < parent_started
    {
        return false;
    }
    match proc_stat(parent_pid) {
        Some(parent) if parent.start_ticks == parent_started => node.ppid == parent_pid,
        // Published parent is gone. Only the orphaned group leader is in scope.
        _ => node.ppid == 1,
    }
}

/// Block until `parent_pid` exits or `stop` is set.
///
/// The owned child is signaled only after [`adopt_reaper_target`] accepts it.
/// On parent exit, runs the same bounded group stop as the `atexit` hook
/// (`grace`, then SIGKILL only if that same start time is still running).
/// `stop` means this reaper was disarmed while it had not yet decided the
/// parent was dead (normal Stop, or a newer reaper replaced this one). The
/// node is not signaled in that case. `grace` is [`STOP_GRACE`] on the
/// production path.
pub fn parent_death_reaper_loop(
    parent_pid: i32,
    parent_started: u64,
    node_pid: i32,
    node_started: u64,
    grace: Duration,
    stop: &AtomicBool,
) {
    if !adopt_reaper_target(parent_pid, parent_started, node_pid, node_started) {
        let msg = format!(
            "node parent-death reaper: not adopting pid {node_pid}; it is not a group-leader child of parent {parent_pid} with the published start times; not signaling\n"
        );
        write_stderr(&msg);
        return;
    }
    let watch = watch_parent(parent_pid, parent_started);
    if matches!(watch, ParentWatch::AlreadyGone) {
        stop_adopted_child(parent_pid, node_pid, node_started, grace);
        return;
    }
    loop {
        if stop.load(Ordering::SeqCst) {
            let msg = format!("node parent-death reaper: disarmed; not signaling {node_pid}\n");
            write_stderr(&msg);
            return;
        }
        if parent_has_exited(parent_pid, parent_started, &watch) {
            stop_adopted_child(parent_pid, node_pid, node_started, grace);
            return;
        }
    }
}

fn stop_adopted_child(parent_pid: i32, node_pid: i32, node_started: u64, grace: Duration) {
    let msg = format!(
        "node parent-death reaper: parent {parent_pid} exited; stopping owned pid {node_pid}\n"
    );
    write_stderr(&msg);
    let outcome = signal_owned_group(node_pid, node_started, grace);
    let msg = format!("node parent-death reaper: stop outcome {outcome:?}\n");
    write_stderr(&msg);
}

/// Run the reaper when `args` starts with `--parent-death-reaper`.
///
/// Returns `None` when this process is the normal GUI. Otherwise the process
/// should exit with the returned code. Installs a SIGTERM handler only on
/// that path so a unit test of the loop does not steal the test process's
/// signals. SIGTERM while the parent is alive disarms the reaper. The stop
/// grace is [`STOP_GRACE`] (10 seconds), the same interval as Stop.
pub fn run_parent_death_reaper_from_args<I>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = String>,
{
    match parse_reaper_args(args) {
        Ok(None) => None,
        Err(err) => {
            let msg = format!("node parent-death reaper: {err}\n");
            write_stderr(&msg);
            Some(2)
        }
        Ok(Some((parent_pid, parent_started, node_pid, node_started))) => {
            unsafe {
                libc::signal(libc::SIGHUP, libc::SIG_IGN);
                libc::signal(
                    libc::SIGTERM,
                    reaper_term as *const () as libc::sighandler_t,
                );
            }
            unblock_sigterm();
            parent_death_reaper_loop(
                parent_pid,
                parent_started,
                node_pid,
                node_started,
                STOP_GRACE,
                &REAPER_PROCESS_STOP,
            );
            Some(0)
        }
    }
}

static REAPER_PROCESS_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn reaper_term(_signal: i32) {
    REAPER_PROCESS_STOP.store(true, Ordering::SeqCst);
}

fn unblock_sigterm() {
    unsafe {
        let mut set = std::mem::zeroed::<libc::sigset_t>();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

fn pidfd_open(pid: i32) -> Option<libc::c_int> {
    if pid <= 0 {
        return None;
    }
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        None
    } else {
        Some(fd as libc::c_int)
    }
}

enum ParentWatch {
    /// pidfd refers to the parent whose start time was published.
    Fd(libc::c_int),
    /// pidfd was not available. Identity is re-checked via start time.
    PollProc,
    /// The published parent is already gone or its pid was reused.
    AlreadyGone,
}

impl Drop for ParentWatch {
    fn drop(&mut self) {
        if let ParentWatch::Fd(fd) = self {
            unsafe {
                libc::close(*fd);
            }
        }
    }
}

fn watch_parent(parent_pid: i32, parent_started: u64) -> ParentWatch {
    match pidfd_open(parent_pid) {
        Some(fd) => match process_start_ticks(parent_pid) {
            Some(ticks) if ticks == parent_started => ParentWatch::Fd(fd),
            _ => {
                unsafe {
                    libc::close(fd);
                }
                ParentWatch::AlreadyGone
            }
        },
        None => match process_start_ticks(parent_pid) {
            Some(ticks) if ticks == parent_started => ParentWatch::PollProc,
            _ => ParentWatch::AlreadyGone,
        },
    }
}

fn parent_has_exited(parent_pid: i32, parent_started: u64, watch: &ParentWatch) -> bool {
    if let ParentWatch::Fd(fd) = watch {
        let mut pollfd = libc::pollfd {
            fd: *fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pollfd, 1, 50) };
        if rc > 0 {
            // Readable pidfd means this process, not a later occupant of the pid, exited.
            return pollfd.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0;
        }
        if rc == 0 {
            return false;
        }
    }
    match process_start_ticks(parent_pid) {
        Some(ticks) if ticks == parent_started => {
            thread::sleep(Duration::from_millis(50));
            false
        }
        _ => true,
    }
}

fn reaper_executable() -> std::io::Result<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(path) = REAPER_EXE_OVERRIDE.lock().expect("reaper exe").clone() {
        return Ok(path);
    }
    std::env::current_exe()
}

fn arm_parent_death_reaper(node_pid: i32, start_ticks: u64) -> Result<(), String> {
    if !PARENT_DEATH_REAPER_ENABLED.load(Ordering::SeqCst) {
        return Ok(());
    }
    disarm_parent_death_reaper();
    let exe = reaper_executable()
        .map_err(|err| format!("node parent-death reaper was not started: {err}"))?;
    let parent_pid = std::process::id();
    let parent_started = process_start_ticks(parent_pid as i32).ok_or(
        "node parent-death reaper was not started: the supervisor start time could not be read",
    )?;
    let mut command = Command::new(exe);
    command
        .arg("--parent-death-reaper")
        .arg(parent_pid.to_string())
        .arg(parent_started.to_string())
        .arg(node_pid.to_string())
        .arg(start_ticks.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    // Safety: runs in the forked child before exec. Block SIGTERM until the
    // reaper installs its handler so a disarm during startup cannot use the
    // default terminate action after the handler would have been required,
    // and put the reaper in its own session so a signal to the GUI group
    // does not kill it before the GUI itself has exited.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            let mut set = std::mem::zeroed::<libc::sigset_t>();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            if libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    match command.spawn() {
        Ok(child) => {
            let pid = child.id();
            eprintln!("node parent-death reaper pid {pid} watching owned pid {node_pid}");
            *REAPER_CHILD.lock().expect("reaper child") = Some(child);
            Ok(())
        }
        Err(err) => Err(format!("node parent-death reaper was not started: {err}")),
    }
}

fn disarm_parent_death_reaper() {
    let child = REAPER_CHILD.lock().expect("reaper child").take();
    let Some(mut child) = child else {
        return;
    };
    let pid = child.id() as i32;
    if pid > 0 {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() >= deadline => {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                let _ = child.wait();
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => return,
        }
    }
}

/// `starttime` from `/proc/pid/stat` (field 22). `None` if that pid is gone.
fn process_start_ticks(pid: i32) -> Option<u64> {
    Some(proc_stat(pid)?.start_ticks)
}

struct ProcStat {
    state: u8,
    ppid: i32,
    pgrp: i32,
    start_ticks: u64,
}

fn proc_stat(pid: i32) -> Option<ProcStat> {
    if pid <= 0 {
        return None;
    }
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rsplit_once(')')?.1;
    // Tokens after `comm` begin at field 3 (state). Field 4 is ppid, 5 is pgrp,
    // 22 is starttime.
    let mut fields = rest.split_whitespace();
    let state = *fields.next()?.as_bytes().first()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgrp = fields.next()?.parse().ok()?;
    let start_ticks = fields.nth(16)?.parse().ok()?;
    Some(ProcStat {
        state,
        ppid,
        pgrp,
        start_ticks,
    })
}

/// Same start time, and not a zombie. A zombie keeps its pid and start time
/// until `wait`, so start time alone would escalate SIGKILL after a clean exit.
fn owned_still_running(pid: i32, started: u64) -> bool {
    matches!(
        proc_stat(pid),
        Some(stat) if stat.start_ticks == started && stat.state != b'Z' && stat.state != b'X'
    )
}

pub extern "C" fn stop_owned_on_process_exit() {
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
    match send_group_signal(pid, started, libc::SIGTERM) {
        GroupSignal::NotOwned => return OwnedSignal::NotOwned,
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
    match send_group_signal(pid, started, libc::SIGKILL) {
        GroupSignal::NotOwned => OwnedSignal::NotOwned,
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
    /// Identity did not match. Nothing was signaled.
    NotOwned,
}

/// Signal `pid`'s process group only while a pidfd holds that exact task and
/// `/proc` still shows the published start time and `pgrp == pid`.
///
/// Holding the pidfd keeps the pid from being reused across the `kill`. A
/// process that is not its own group leader is refused: `kill(-pid)` would
/// name some other group whose id happens to equal this pid.
fn send_group_signal(pid: i32, started: u64, signal: i32) -> GroupSignal {
    if pid <= 0 {
        return GroupSignal::Failed;
    }
    #[cfg(test)]
    if EXIT_SIGNAL_FAILS.load(Ordering::SeqCst) {
        return GroupSignal::Failed;
    }
    let Some(fd) = pidfd_open(pid) else {
        return GroupSignal::Gone;
    };
    struct CloseFd(libc::c_int);
    impl Drop for CloseFd {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.0);
            }
        }
    }
    let _hold = CloseFd(fd);
    let Some(stat) = proc_stat(pid) else {
        return GroupSignal::Gone;
    };
    if stat.start_ticks != started || stat.pgrp != pid {
        return GroupSignal::NotOwned;
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
    use std::sync::Arc;

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

    /// Parent process whose forked child is in its own session. The child's
    /// ppid is the parent, which is what the reaper requires before it adopts.
    fn spawn_parent_child(binary: &Path, pid_file: &Path) -> (Child, i32) {
        let mut parent = Command::new("python3")
            .arg("-c")
            .arg(
                "import os, sys, time\nbinary, pid_file = sys.argv[1], sys.argv[2]\npid = os.fork()\nif pid == 0:\n    os.setsid()\n    os.execv(binary, [binary])\nopen(pid_file, 'w').write(str(pid))\nwhile True:\n    time.sleep(0.05)\n",
            )
            .arg(binary)
            .arg(pid_file)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(text) = fs::read_to_string(pid_file) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    if proc_stat(pid)
                        .is_some_and(|stat| stat.ppid == parent.id() as i32 && stat.pgrp == pid)
                    {
                        return (parent, pid);
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "parent did not publish a group-leader child"
            );
            match parent.try_wait() {
                Ok(Some(_)) => panic!("parent exited before publishing a child"),
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(_) => panic!("could not wait for the parent"),
            }
        }
    }

    fn cmdline_contains(needle: &str) -> bool {
        let Ok(entries) = fs::read_dir("/proc") else {
            return false;
        };
        for entry in entries.flatten() {
            let Ok(bytes) = fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            if String::from_utf8_lossy(&bytes).contains(needle) {
                return true;
            }
        }
        false
    }

    struct ReaperTestGuard;
    impl ReaperTestGuard {
        fn enable(exe: PathBuf) -> Self {
            *REAPER_EXE_OVERRIDE.lock().expect("reaper exe") = Some(exe);
            PARENT_DEATH_REAPER_ENABLED.store(true, Ordering::SeqCst);
            ReaperTestGuard
        }
    }
    impl Drop for ReaperTestGuard {
        fn drop(&mut self) {
            PARENT_DEATH_REAPER_ENABLED.store(false, Ordering::SeqCst);
            *REAPER_EXE_OVERRIDE.lock().expect("reaper exe") = None;
            disarm_parent_death_reaper();
        }
    }

    static PROCESS_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        PROCESS_TEST_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    static MOCK_PREVIOUS_CALLED: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);
    static LAST_MOCK_ERROR_CODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    static LAST_MOCK_RESOURCE_ID: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    unsafe extern "C" fn mock_previous_handler(
        _display: *mut std::ffi::c_void,
        event: *mut XErrorEvent,
    ) -> i32 {
        MOCK_PREVIOUS_CALLED.fetch_add(1, Ordering::SeqCst);
        if !event.is_null() {
            LAST_MOCK_ERROR_CODE.store((*event).error_code, Ordering::SeqCst);
            LAST_MOCK_RESOURCE_ID.store((*event).resourceid, Ordering::SeqCst);
        }
        42
    }

    /// Inject an X error through [`dispatch_x_error`], the production callback body.
    fn inject_x_error(error_code: u8, resourceid: usize) -> i32 {
        let mut event = XErrorEvent {
            type_: 0,
            display: std::ptr::null_mut(),
            resourceid,
            serial: 1,
            error_code,
            request_code: 14,
            minor_code: 0,
        };
        unsafe {
            dispatch_x_error(
                std::ptr::null_mut(),
                &mut event,
                Some(mock_previous_handler),
            )
        }
    }

    fn reset_x11(tracked_xid: usize) {
        disarm_window_teardown();
        set_tracked_window_xid(tracked_xid);
        clear_gdk_dispatch();
        MOCK_PREVIOUS_CALLED.store(0, Ordering::SeqCst);
        LAST_MOCK_ERROR_CODE.store(0, Ordering::SeqCst);
        LAST_MOCK_RESOURCE_ID.store(0, Ordering::SeqCst);
    }

    /// Same sequence as `on_gdk_event`: record the event, arm teardown only when
    /// [`gdk_event_starts_teardown_for_window`] says so, then the caller invokes
    /// [`dispatch_x_error`] and [`clear_gdk_dispatch`].
    fn dispatch_tracked_gdk_event(event_type: i32, changed_mask: i32, new_window_state: i32) {
        note_gdk_dispatch(event_type, changed_mask, new_window_state);
        if gdk_event_starts_teardown_for_window(event_type, new_window_state, true) {
            note_window_teardown();
        }
    }

    #[test]
    fn x_errors_are_ignored_only_for_teardown_drawable_codes() {
        let _lock = test_lock();
        disarm_window_teardown();
        assert!(!window_teardown_started());
        for code in [0u8, 1, 2, 3, 8, 9, 10, 11, 255] {
            assert!(
                !ignore_x_error_during_teardown(code, false),
                "code {code} was ignored before teardown"
            );
            assert!(
                !should_suppress_x_error(code, 0x200003, false, 0x200003),
                "code {code} was suppressed before teardown"
            );
        }
        assert!(ignore_x_error_during_teardown(X_BAD_WINDOW, true));
        assert!(ignore_x_error_during_teardown(X_BAD_DRAWABLE, true));
        assert!(should_suppress_x_error(
            X_BAD_WINDOW,
            0x200003,
            true,
            0x200003
        ));
        assert!(should_suppress_x_error(
            X_BAD_DRAWABLE,
            0x200003,
            true,
            0x200003
        ));
        assert!(
            !should_suppress_x_error(X_BAD_WINDOW, 0x999999, true, 0x200003),
            "unrelated resource was suppressed"
        );
        assert!(
            !should_suppress_x_error(X_BAD_DRAWABLE, 0x200003, true, 0),
            "teardown without a tracked xid was suppressed"
        );
        for code in [0u8, 1, 2, 8, 10, 11, 255] {
            assert!(
                !ignore_x_error_during_teardown(code, true),
                "code {code} was ignored during teardown"
            );
            assert!(
                !should_suppress_x_error(code, 0x200003, true, 0x200003),
                "code {code} was suppressed during teardown"
            );
        }
        // GDK_DESTROY represents actual destruction
        assert!(gdk_event_starts_teardown(GDK_DESTROY, 0));
        assert!(gdk_event_starts_teardown_for_window(GDK_DESTROY, 0, true));
        assert!(!gdk_event_starts_teardown_for_window(GDK_DESTROY, 0, false));

        // Window state changes (including minimize, restore, and withdraw) NEVER arm teardown
        assert!(!gdk_event_starts_teardown(GDK_WINDOW_STATE, 1));
        assert!(!gdk_event_starts_teardown(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED
        ));
        assert!(!gdk_event_starts_teardown(
            GDK_WINDOW_STATE,
            (1 << 7) | GDK_WINDOW_STATE_ICONIFIED
        ));
        assert!(!gdk_event_starts_teardown(GDK_WINDOW_STATE, 1 << 7));
        assert!(!gdk_event_starts_teardown(GDK_WINDOW_STATE, 0));
        assert!(!gdk_event_starts_teardown_for_window(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED,
            true
        ));
        assert!(!gdk_event_starts_teardown_for_window(
            GDK_WINDOW_STATE,
            0,
            true
        ));
        assert!(!gdk_event_starts_teardown(0, 0));
        assert!(!gdk_event_starts_teardown(2, 1));
        assert!(!gdk_event_starts_teardown(12, 1));
    }

    #[test]
    fn regression_minimize_and_restore_do_not_arm_teardown_or_suppress_errors() {
        let _lock = test_lock();
        let tracked_xid = 0x200003;
        let aux_xid = 0x999999;
        reset_x11(tracked_xid);

        // Injected while the production dispatch still has the minimize event.
        // changed_mask bit 1 is ICONIFIED. This is not an approved close.
        dispatch_tracked_gdk_event(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED,
            GDK_WINDOW_STATE_ICONIFIED,
        );
        assert_eq!(current_gdk_event(), GDK_WINDOW_STATE);
        assert_eq!(
            gdk_window_state_snapshot().0 & GDK_WINDOW_STATE_ICONIFIED,
            GDK_WINDOW_STATE_ICONIFIED
        );
        assert!(!window_teardown_started(), "minimize must not arm teardown");

        let ret = inject_x_error(X_BAD_WINDOW, tracked_xid);
        assert_eq!(
            ret, 42,
            "BadWindow on the tracked window must be forwarded during minimize"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 1);
        assert_eq!(LAST_MOCK_RESOURCE_ID.load(Ordering::SeqCst), tracked_xid);
        assert!(!window_teardown_started());

        let ret = inject_x_error(X_BAD_DRAWABLE, tracked_xid);
        assert_eq!(
            ret, 42,
            "BadDrawable on the tracked window must be forwarded during minimize"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 2);

        let ret = inject_x_error(X_BAD_WINDOW, aux_xid);
        assert_eq!(ret, 42, "BadWindow on an unrelated xid must be forwarded");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 3);
        assert_eq!(LAST_MOCK_RESOURCE_ID.load(Ordering::SeqCst), aux_xid);

        let ret = inject_x_error(10, tracked_xid);
        assert_eq!(ret, 42, "ordinary error must be forwarded while minimized");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 4);
        clear_gdk_dispatch();
        assert_eq!(current_gdk_event(), -1);

        // Restore also changes the iconified bit. new_window_state drops it.
        dispatch_tracked_gdk_event(GDK_WINDOW_STATE, GDK_WINDOW_STATE_ICONIFIED, 0);
        assert!(!window_teardown_started(), "restore must not arm teardown");
        let ret = inject_x_error(X_BAD_DRAWABLE, tracked_xid);
        assert_eq!(
            ret, 42,
            "BadDrawable on the tracked window must be forwarded during restore"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 5);
        assert!(!window_teardown_started());
        clear_gdk_dispatch();
    }

    #[test]
    fn regression_stale_window_state_does_not_attribute_a_later_x_error() {
        let _lock = test_lock();
        let tracked_xid = 0x200003;
        reset_x11(tracked_xid);
        dispatch_tracked_gdk_event(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED,
            GDK_WINDOW_STATE_ICONIFIED,
        );
        // Production `on_gdk_event` clears this when `gtk_main_do_event` returns.
        clear_gdk_dispatch();
        assert_eq!(current_gdk_event(), -1);
        assert_eq!(gdk_window_state_snapshot(), (0, 0));
        assert!(!window_teardown_started());

        let ret = inject_x_error(X_BAD_WINDOW, tracked_xid);
        assert_eq!(
            ret, 42,
            "a BadWindow after minimize dispatch must not be swallowed"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 1);
        assert!(!window_teardown_started());
    }

    #[test]
    fn regression_cancelled_close_disarms_teardown_and_preserves_error_handling() {
        let _lock = test_lock();
        let tracked_xid = 0x200003;
        reset_x11(tracked_xid);
        let tracked = 0x10 as *mut std::ffi::c_void;
        let other = 0x11 as *mut std::ffi::c_void;
        assert!(delete_event_targets_tracked(tracked, tracked));
        assert!(!delete_event_targets_tracked(tracked, other));
        assert!(delete_event_targets_tracked(std::ptr::null_mut(), other));
        // Production delete-event hook arms teardown only for the tracked widget.
        if delete_event_targets_tracked(tracked, tracked) {
            note_window_teardown();
        }
        assert!(window_teardown_started());
        // StayOpen: close was not confirmed.
        disarm_window_teardown();
        assert!(
            !window_teardown_started(),
            "teardown must be disarmed on cancelled close"
        );

        let ret = inject_x_error(X_BAD_WINDOW, tracked_xid);
        assert_eq!(
            ret, 42,
            "BadWindow on the tracked window must be forwarded after cancelled close"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 1);
        let ret = inject_x_error(X_BAD_DRAWABLE, 0x999999);
        assert_eq!(ret, 42, "unrelated BadDrawable must be forwarded");
        assert_eq!(LAST_MOCK_RESOURCE_ID.load(Ordering::SeqCst), 0x999999);
        let ret = inject_x_error(10, tracked_xid);
        assert_eq!(
            ret, 42,
            "ordinary error must be forwarded after cancelled close"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn regression_auxiliary_window_events_do_not_arm_teardown() {
        let _lock = test_lock();
        let tracked_xid = 0x200003;
        let aux_xid = 0x999999;
        reset_x11(tracked_xid);

        assert!(
            !gdk_event_starts_teardown_for_window(GDK_DESTROY, 0, false),
            "auxiliary window destroy must not arm teardown"
        );
        assert!(
            !gdk_event_starts_teardown_for_window(0, 0, false),
            "auxiliary window delete must not arm teardown"
        );
        note_gdk_dispatch(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED,
            GDK_WINDOW_STATE_ICONIFIED,
        );
        // Auxiliary window: production does not call note_window_teardown.
        assert!(!gdk_event_starts_teardown_for_window(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED,
            false
        ));
        assert!(!window_teardown_started());
        let ret = inject_x_error(X_BAD_WINDOW, aux_xid);
        assert_eq!(ret, 42, "auxiliary window error must be forwarded");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 1);
        assert_eq!(LAST_MOCK_RESOURCE_ID.load(Ordering::SeqCst), aux_xid);
        clear_gdk_dispatch();
    }

    #[test]
    fn regression_ordinary_non_teardown_x_errors_forwarded_to_previous_handler() {
        let _lock = test_lock();
        let tracked_xid = 0x200003;
        reset_x11(tracked_xid);

        let codes = [1u8, 2, 3, 8, 9, 10, 11, 255];
        let mut expected_calls = 0;
        for &code in &codes {
            let ret = inject_x_error(code, tracked_xid);
            expected_calls += 1;
            assert_eq!(
                ret, 42,
                "code {code} must be forwarded to previous handler outside teardown"
            );
            assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), expected_calls);
            assert_eq!(LAST_MOCK_ERROR_CODE.load(Ordering::SeqCst), code);
        }

        note_window_teardown();
        let non_drawable_codes = [1u8, 2, 8, 10, 11, 255];
        for &code in &non_drawable_codes {
            let ret = inject_x_error(code, tracked_xid);
            expected_calls += 1;
            assert_eq!(
                ret, 42,
                "code {code} must be forwarded to previous handler even during teardown"
            );
            assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), expected_calls);
            assert_eq!(LAST_MOCK_ERROR_CODE.load(Ordering::SeqCst), code);
        }
        disarm_window_teardown();
    }

    #[test]
    fn regression_real_close_suppresses_only_tracked_drawable_errors() {
        let _lock = test_lock();
        let tracked_xid = 0x200003;
        let aux_xid = 0x999999;
        reset_x11(tracked_xid);

        // Approved close (AllowExit) and tracked GDK_DESTROY both arm teardown.
        dispatch_tracked_gdk_event(GDK_DESTROY, 0, 0);
        assert!(window_teardown_started());
        note_window_teardown();

        let ret = inject_x_error(X_BAD_WINDOW, tracked_xid);
        assert_eq!(ret, 0, "BadWindow on the tracked xid must be suppressed");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 0);

        let ret = inject_x_error(X_BAD_DRAWABLE, tracked_xid);
        assert_eq!(ret, 0, "BadDrawable on the tracked xid must be suppressed");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 0);

        let ret = inject_x_error(X_BAD_WINDOW, aux_xid);
        assert_eq!(
            ret, 42,
            "BadWindow on an unrelated xid must be forwarded during teardown"
        );
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 1);
        assert_eq!(LAST_MOCK_RESOURCE_ID.load(Ordering::SeqCst), aux_xid);

        let ret = inject_x_error(X_BAD_DRAWABLE, aux_xid);
        assert_eq!(ret, 42, "BadDrawable on an unrelated xid must be forwarded");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 2);

        let ret = inject_x_error(10, tracked_xid);
        assert_eq!(ret, 42, "BadAccess must be forwarded even during teardown");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 3);
        clear_gdk_dispatch();
        disarm_window_teardown();
    }

    #[test]
    fn regression_returning_previous_handler_does_not_stop_the_owned_child() {
        let _lock = test_lock();
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
        let _bystander_guard = KillPid(bystander_pid);
        let supervisor = Supervisor::new(Duration::from_millis(400));
        supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap();
        let owned = supervisor.status().pid.expect("owned pid");
        let _owned_guard = KillPid(owned);
        assert_eq!(OWNED_PID.load(Ordering::SeqCst), owned as i32);
        assert!(process_alive(owned));
        assert!(process_alive(bystander_pid));

        let tracked_xid = 0x200003;
        reset_x11(tracked_xid);
        dispatch_tracked_gdk_event(
            GDK_WINDOW_STATE,
            GDK_WINDOW_STATE_ICONIFIED,
            GDK_WINDOW_STATE_ICONIFIED,
        );
        let ret = inject_x_error(X_BAD_DRAWABLE, tracked_xid);
        assert_eq!(ret, 42, "returning handler result must be propagated");
        assert_eq!(MOCK_PREVIOUS_CALLED.load(Ordering::SeqCst), 1);
        assert_eq!(
            OWNED_PID.load(Ordering::SeqCst),
            owned as i32,
            "a returning previous handler cleared the owned pid"
        );
        assert!(
            process_alive(owned),
            "a returning previous handler stopped the owned child"
        );
        assert!(process_alive(bystander_pid));
        assert_eq!(supervisor.status().state, ProcessState::Running);
        clear_gdk_dispatch();
        assert!(!window_teardown_started());

        // No previous handler still must not stop the child.
        let mut event = XErrorEvent {
            type_: 0,
            display: std::ptr::null_mut(),
            resourceid: tracked_xid,
            serial: 1,
            error_code: X_BAD_WINDOW,
            request_code: 14,
            minor_code: 0,
        };
        let ret = unsafe { dispatch_x_error(std::ptr::null_mut(), &mut event, None) };
        assert_eq!(ret, 0);
        assert!(process_alive(owned));
        assert_eq!(OWNED_PID.load(Ordering::SeqCst), owned as i32);

        supervisor.stop().unwrap();
        assert!(!process_alive(owned));
        assert!(process_alive(bystander_pid));
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn parent_death_reaper_stops_owned_child_and_spares_bystander() {
        let _lock = test_lock();
        assert_eq!(STOP_GRACE, Duration::from_secs(10));
        assert!(parse_reaper_args(std::iter::empty()).unwrap().is_none());
        assert!(parse_reaper_args(["--other".to_string()].into_iter())
            .unwrap()
            .is_none());
        assert!(parse_reaper_args(
            ["--parent-death-reaper".to_string(), "1".to_string()].into_iter()
        )
        .is_err());
        assert!(parse_reaper_args(
            [
                "--parent-death-reaper".to_string(),
                "2".to_string(),
                "10".to_string(),
                "2".to_string(),
                "11".to_string(),
            ]
            .into_iter()
        )
        .is_err());

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
        let (mut parent, owned_pid) = spawn_parent_child(&hold, &tmp.path().join("child.pid"));
        let parent_pid = parent.id() as i32;
        let parent_started = process_start_ticks(parent_pid).expect("parent start");
        let owned_started = process_start_ticks(owned_pid).expect("owned start");
        let mut bystander = spawn_session(&other);
        let bystander_pid = bystander.id();
        let _bystander_guard = KillPid(bystander_pid);
        let _owned_guard = KillPid(owned_pid as u32);

        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let early = thread::spawn(move || {
            parent_death_reaper_loop(
                parent_pid,
                parent_started,
                owned_pid,
                owned_started,
                Duration::from_millis(200),
                &flag,
            );
        });
        thread::sleep(Duration::from_millis(120));
        assert!(
            process_alive(owned_pid as u32),
            "reaper stopped a live parent's child"
        );
        assert!(process_alive(bystander_pid));
        stop.store(true, Ordering::SeqCst);
        early.join().unwrap();
        assert!(process_alive(owned_pid as u32));
        assert!(process_alive(bystander_pid));

        let flag = Arc::new(AtomicBool::new(false));
        let watcher = thread::spawn(move || {
            parent_death_reaper_loop(
                parent_pid,
                parent_started,
                owned_pid,
                owned_started,
                Duration::from_millis(400),
                &flag,
            );
        });
        unsafe {
            libc::kill(parent_pid, libc::SIGKILL);
        }
        let _ = parent.wait();
        watcher.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_alive(owned_pid as u32) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !owned_still_running(owned_pid, owned_started),
            "parent death did not stop the owned child"
        );
        assert!(
            process_alive(bystander_pid),
            "parent-death reaper killed a bystander"
        );
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn reaper_refuses_unrelated_pid_reused_ticks_and_non_leader() {
        let _lock = test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.sh",
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let mut non_leader = Command::new(&hold).spawn().unwrap();
        let non_leader_pid = non_leader.id() as i32;
        let non_leader_started = process_start_ticks(non_leader_pid).expect("non leader start");
        let began = Instant::now();
        let outcome =
            signal_owned_group(non_leader_pid, non_leader_started, Duration::from_secs(10));
        assert_eq!(outcome, OwnedSignal::NotOwned);
        assert!(
            began.elapsed() < Duration::from_millis(500),
            "a non-leader was treated as an owned group and the grace was consumed"
        );
        assert!(process_alive(non_leader.id()));
        let _ = non_leader.kill();
        let _ = non_leader.wait();

        let mut leader = spawn_session(&hold);
        let leader_pid = leader.id() as i32;
        let leader_started = process_start_ticks(leader_pid).expect("leader start");
        let outcome = signal_owned_group(
            leader_pid,
            leader_started.wrapping_add(1),
            Duration::from_secs(10),
        );
        assert_eq!(outcome, OwnedSignal::NotOwned);
        assert!(
            process_alive(leader.id()),
            "wrong start time signaled a live group"
        );

        let (mut parent, owned_pid) = spawn_parent_child(&hold, &tmp.path().join("child.pid"));
        let _owned_guard = KillPid(owned_pid as u32);
        let parent_pid = parent.id() as i32;
        let parent_started = process_start_ticks(parent_pid).expect("parent start");
        let by_started = leader_started;
        let by_pid = leader_pid;
        let stop = AtomicBool::new(false);
        parent_death_reaper_loop(
            parent_pid,
            parent_started,
            by_pid,
            by_started,
            Duration::from_millis(200),
            &stop,
        );
        assert!(
            process_alive(leader.id()),
            "reaper adopted a group leader that is not the parent's child"
        );
        unsafe {
            libc::kill(parent_pid, libc::SIGKILL);
        }
        let _ = parent.wait();
        assert!(process_alive(leader.id()));
        let _ = leader.kill();
        let _ = leader.wait();
    }

    #[test]
    fn reaper_adopts_child_already_orphaned_by_parent_death() {
        let _lock = test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.sh",
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let (mut parent, owned_pid) = spawn_parent_child(&hold, &tmp.path().join("child.pid"));
        let _owned_guard = KillPid(owned_pid as u32);
        let parent_pid = parent.id() as i32;
        let parent_started = process_start_ticks(parent_pid).expect("parent start");
        let owned_started = process_start_ticks(owned_pid).expect("owned start");
        let mut bystander = spawn_session(&script(
            tmp.path(),
            "bystander.sh",
            "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
        ));
        let bystander_pid = bystander.id();
        unsafe {
            libc::kill(parent_pid, libc::SIGKILL);
        }
        let _ = parent.wait();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let ppid = proc_stat(owned_pid).map(|stat| stat.ppid);
            if ppid == Some(1) || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            proc_stat(owned_pid).map(|stat| stat.ppid),
            Some(1),
            "orphaned child was not reparented to init"
        );
        let stop = AtomicBool::new(false);
        parent_death_reaper_loop(
            parent_pid,
            parent_started,
            owned_pid,
            owned_started,
            Duration::from_millis(400),
            &stop,
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_alive(owned_pid as u32) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !owned_still_running(owned_pid, owned_started),
            "a child orphaned before the reaper loop started was left running"
        );
        assert!(process_alive(bystander_pid));
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn reaper_spawn_failure_stops_the_child_and_restart_replaces_the_reaper() {
        let _lock = test_lock();
        let tmp = tempfile::tempdir().unwrap();
        let hold = script(
            tmp.path(),
            "hold.sh",
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
        );
        let log = tmp.path().join("reaper.log");
        let reaper = script(
            tmp.path(),
            "reaper.py",
            &format!(
                "#!/usr/bin/env python3\nimport os, signal, sys, time\nsignal.pthread_sigmask(signal.SIG_UNBLOCK, {{signal.SIGTERM}})\nlog = {log:?}\ndef stop(_s, _f):\n    open(log, 'a').write('term %d\\n' % os.getpid())\n    sys.exit(0)\nsignal.signal(signal.SIGTERM, stop)\nopen(log, 'a').write('start %d %s\\n' % (os.getpid(), ' '.join(sys.argv[1:])))\nwhile True:\n    time.sleep(0.05)\n"
            ),
        );
        let _guard = ReaperTestGuard::enable(tmp.path().join("missing-reaper"));
        let supervisor = Supervisor::new(Duration::from_millis(300));
        let err = supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap_err();
        assert!(err.contains("parent-death reaper was not started"), "{err}");
        assert!(err.contains("The child was stopped"), "{err}");
        let status = supervisor.status();
        assert_ne!(status.state, ProcessState::Running);
        assert!(status.pid.is_none());
        assert_eq!(OWNED_PID.load(Ordering::SeqCst), 0);
        assert!(!cmdline_contains(hold.to_str().unwrap()));

        drop(_guard);
        let _guard = ReaperTestGuard::enable(reaper);
        let supervisor = Supervisor::new(Duration::from_millis(400));
        supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap();
        let first = supervisor.status().pid.expect("first pid");
        let _first_guard = KillPid(first);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if fs::read_to_string(&log)
                .unwrap_or_default()
                .contains(&format!(" {first} "))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "reaper did not record the first child"
            );
            thread::sleep(Duration::from_millis(20));
        }
        supervisor.stop().unwrap();
        assert!(!process_alive(first));
        let after_stop = fs::read_to_string(&log).unwrap();
        assert!(after_stop.contains("term "), "{after_stop}");

        supervisor
            .start(&hold, Path::new("/tmp/unused.toml"))
            .unwrap();
        let second = supervisor.status().pid.expect("second pid");
        let _second_guard = KillPid(second);
        assert_ne!(first, second);
        assert!(process_alive(second));
        assert!(!process_alive(first));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let text = fs::read_to_string(&log).unwrap_or_default();
            if text.matches("start ").count() >= 2 && text.contains(&format!(" {second} ")) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "restart did not arm a new reaper: {text}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        supervisor.stop().unwrap();
        assert!(!process_alive(second));
        assert_eq!(supervisor.status().state, ProcessState::Exited);
    }

    #[test]
    fn spawn_failure_does_not_look_running() {
        let _lock = test_lock();
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
        let _lock = test_lock();
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
        let _lock = test_lock();
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
        let _lock = test_lock();
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
        let _lock = test_lock();
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
        let _lock = test_lock();
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
        let _lock = test_lock();
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
        let _lock = test_lock();
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

        let mut held = spawn_session(&stubborn);
        let held_pid = held.id() as i32;
        let held_guard = KillPid(held.id());
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
        drop(held_guard);
        let _ = held.wait();
        let _ = bystander.kill();
        let _ = bystander.wait();
    }
}
