//! Kovanica Node milestone 1: Linux supervisor and read-only monitor.
//!
//! The webview cannot start a process except through these commands.
//! There is no remote admin endpoint and no transaction submission.

use std::time::Instant;

use kovanica_node_desktop::monitor::fetch_monitor;
use kovanica_node_desktop::policy::{check_setup, kovanica_env_names};
use kovanica_node_desktop::session::{BeginPoll, Session, Snapshot, WindowClose};
use kovanica_node_desktop::supervisor::{arm_exit_hook, ProcessState, STOP_GRACE};
use tauri::Manager;
use tauri::State;

/// BadDrawable while WebKit tears down a window on Xvfb. The default GDK
/// handler can `exit` before `CloseRequested`. Ignore only that error so the
/// owned-child stop can run. Other errors still go to the previous handler.
#[repr(C)]
#[allow(dead_code)]
struct XErrorEvent {
    _type: i32,
    display: *mut std::ffi::c_void,
    resourceid: usize,
    serial: usize,
    error_code: u8,
    request_code: u8,
    minor_code: u8,
}

type XErrorHandler = unsafe extern "C" fn(*mut std::ffi::c_void, *mut XErrorEvent) -> i32;

#[link(name = "X11")]
extern "C" {
    fn XSetErrorHandler(handler: Option<XErrorHandler>) -> Option<XErrorHandler>;
}

static mut PREVIOUS_X_ERROR: Option<XErrorHandler> = None;

unsafe extern "C" fn ignore_teardown_drawable(
    display: *mut std::ffi::c_void,
    event: *mut XErrorEvent,
) -> i32 {
    let code = unsafe { (*event).error_code };
    // 3 BadWindow, 9 BadDrawable. Both show up while the webview window is destroyed.
    if code == 3 || code == 9 {
        return 0;
    }
    unsafe {
        if let Some(previous) = PREVIOUS_X_ERROR {
            return previous(display, event);
        }
    }
    0
}

fn install_x_error_guard() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        PREVIOUS_X_ERROR = XSetErrorHandler(Some(ignore_teardown_drawable));
    });
}

fn current_kovanica_env() -> Vec<String> {
    let keys: Vec<String> = std::env::vars().map(|(key, _)| key).collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    kovanica_env_names(refs)
}

fn paths<'a>(
    binary: &'a str,
    config_path: &'a str,
    data_dir: &'a str,
) -> (
    &'a std::path::Path,
    &'a std::path::Path,
    &'a std::path::Path,
) {
    (
        std::path::Path::new(binary),
        std::path::Path::new(config_path),
        std::path::Path::new(data_dir),
    )
}

#[tauri::command]
fn inspect_setup(
    state: State<'_, Session>,
    binary: String,
    config_path: String,
    data_dir: String,
) -> Result<kovanica_node_desktop::policy::CheckedSetup, String> {
    let (binary, config_path, data_dir) = paths(&binary, &config_path, &data_dir);
    let checked = check_setup(binary, config_path, data_dir, &current_kovanica_env())?;
    state.store_setup(checked.clone());
    Ok(checked)
}

#[tauri::command]
fn node_start(
    state: State<'_, Session>,
    binary: String,
    config_path: String,
    data_dir: String,
) -> Result<Snapshot, String> {
    let (binary, config_path, data_dir) = paths(&binary, &config_path, &data_dir);
    let checked = check_setup(binary, config_path, data_dir, &current_kovanica_env())?;
    state.supervisor.start(
        std::path::Path::new(&checked.binary),
        std::path::Path::new(&checked.config_path),
    )?;
    state.attach_checked(checked);
    Ok(state.snapshot())
}

#[tauri::command]
fn node_stop(state: State<'_, Session>) -> Result<Snapshot, String> {
    state.stop_owned()?;
    Ok(state.snapshot())
}

#[tauri::command]
fn node_status(state: State<'_, Session>) -> Snapshot {
    state.snapshot()
}

#[tauri::command]
fn set_monitor_paused(state: State<'_, Session>, paused: bool) {
    state.set_paused(paused);
}

#[tauri::command]
async fn monitor_refresh(state: State<'_, Session>) -> Result<Snapshot, String> {
    match state.begin_monitor_poll(Instant::now()) {
        BeginPoll::Idle(snapshot) => Ok(*snapshot),
        BeginPoll::Fetch {
            generation,
            rpc_http,
        } => {
            match fetch_monitor(&rpc_http).await {
                Ok(view) => state.finish_monitor_ok(generation, Instant::now(), view),
                Err(err) => state.finish_monitor_err(generation, Instant::now(), err),
            }
            Ok(state.snapshot())
        }
    }
}

/// Stop the owned child. Stay open when that stop is not confirmed.
fn apply_close(state: &Session) -> WindowClose {
    eprintln!("node close requested");
    let decision = state.close_window();
    if let WindowClose::StayOpen { error } = &decision {
        eprintln!("node stop was not confirmed; keeping the window open: {error}");
    }
    decision
}

fn main() {
    let app = Session::new(STOP_GRACE);
    tauri::Builder::default()
        .manage(app)
        .invoke_handler(tauri::generate_handler![
            inspect_setup,
            node_start,
            node_stop,
            node_status,
            set_monitor_paused,
            monitor_refresh
        ])
        .build(tauri::generate_context!())
        .expect("error while building Kovanica Node")
        .run(|app, event| {
            // After GTK has registered its own handlers, so this one runs first.
            arm_exit_hook();
            install_x_error_guard();
            match event {
                tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    if let Some(state) = app.try_state::<Session>() {
                        if let WindowClose::StayOpen { .. } = apply_close(state.inner()) {
                            api.prevent_close();
                        }
                    }
                }
                tauri::RunEvent::ExitRequested { api, .. } => {
                    if let Some(state) = app.try_state::<Session>() {
                        // If the window close already confirmed the stop, this returns Ok.
                        // If it did not, do not leave the process while the child may be alive.
                        if matches!(
                            state.inner().supervisor.status().state,
                            ProcessState::Running | ProcessState::Starting | ProcessState::Stopping
                        ) {
                            if let WindowClose::StayOpen { .. } = apply_close(state.inner()) {
                                api.prevent_exit();
                            }
                        }
                    }
                }
                _ => {}
            }
        });
}
