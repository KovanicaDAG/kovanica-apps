//! Kovanica Node milestone 1: Linux supervisor and read-only monitor.
//!
//! The webview cannot start a process except through these commands.
//! There is no remote admin endpoint and no transaction submission.

use std::time::Instant;

use kovanica_node_desktop::monitor::fetch_monitor;
use kovanica_node_desktop::policy::{check_setup, kovanica_env_names};
use kovanica_node_desktop::session::{BeginPoll, Session, Snapshot, WindowClose};
use kovanica_node_desktop::supervisor::{
    arm_exit_hook, clear_gdk_dispatch, delete_event_targets_tracked, disarm_window_teardown,
    dispatch_x_error, enable_parent_death_reaper, gdk_event_starts_teardown_for_window,
    note_gdk_dispatch, note_window_teardown, run_parent_death_reaper_from_args,
    set_tracked_window_xid, ProcessState, XErrorEvent, XErrorHandler, GDK_WINDOW_STATE, STOP_GRACE,
};
use tauri::Manager;
use tauri::State;

#[link(name = "X11")]
extern "C" {
    fn XSetErrorHandler(handler: Option<XErrorHandler>) -> Option<XErrorHandler>;
}

#[link(name = "gdk-3")]
extern "C" {
    fn gdk_event_handler_set(
        func: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void)>,
        data: *mut std::ffi::c_void,
        notify: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    );
    fn gdk_window_get_window_type(window: *mut std::ffi::c_void) -> i32;
    fn gdk_x11_window_get_xid(window: *mut std::ffi::c_void) -> usize;
}

#[link(name = "gobject-2.0")]
extern "C" {
    fn g_signal_lookup(name: *const libc::c_char, itype: usize) -> u32;
    fn g_signal_add_emission_hook(
        signal_id: u32,
        detail: u32,
        hook: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            u32,
            *const std::ffi::c_void,
            *mut std::ffi::c_void,
        ) -> i32,
        data: *mut std::ffi::c_void,
        destroy: Option<unsafe extern "C" fn(*mut std::ffi::c_void)>,
    ) -> u64;
}

#[link(name = "gtk-3")]
extern "C" {
    fn gtk_widget_get_type() -> usize;
    fn gtk_widget_get_window(widget: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn gtk_main_do_event(event: *mut std::ffi::c_void);
}

fn write_stderr(msg: &str) {
    unsafe {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
}

/// `GdkEventType` of the event currently inside `gtk_main_do_event` lives in
/// the supervisor crate so tests call the same dispatch the handler uses.
static TRACKED_GTK_WINDOW: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static TRACKED_GDK_WINDOW: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[repr(C)]
#[allow(dead_code)]
struct GdkEventAny {
    event_type: i32,
    window: *mut std::ffi::c_void,
    send_event: i8,
}

#[repr(C)]
#[allow(dead_code)]
struct GdkEventWindowState {
    event_type: i32,
    window: *mut std::ffi::c_void,
    send_event: i8,
    changed_mask: i32,
    new_window_state: i32,
}

const GDK_WINDOW_TOPLEVEL: i32 = 1;

unsafe extern "C" fn on_gdk_event(event: *mut std::ffi::c_void, _data: *mut std::ffi::c_void) {
    let kind = if event.is_null() {
        -1
    } else {
        unsafe { *(event as *const i32) }
    };

    let event_win = if event.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*(event as *const GdkEventAny)).window }
    };

    if !event_win.is_null()
        && TRACKED_GDK_WINDOW
            .load(std::sync::atomic::Ordering::SeqCst)
            .is_null()
        && unsafe { gdk_window_get_window_type(event_win) } == GDK_WINDOW_TOPLEVEL
    {
        TRACKED_GDK_WINDOW.store(event_win, std::sync::atomic::Ordering::SeqCst);
        let xid = unsafe { gdk_x11_window_get_xid(event_win) };
        set_tracked_window_xid(xid);
    }

    let tracked = TRACKED_GDK_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
    let is_tracked = !tracked.is_null() && event_win == tracked;

    let mut changed_mask = 0;
    let mut new_state = 0;
    if kind == GDK_WINDOW_STATE && !event.is_null() {
        let state = unsafe { &*(event as *const GdkEventWindowState) };
        changed_mask = state.changed_mask;
        new_state = state.new_window_state;
    }
    note_gdk_dispatch(kind, changed_mask, new_state);

    if gdk_event_starts_teardown_for_window(kind, new_state, is_tracked) {
        note_window_teardown();
        write_stderr(&format!(
            "node teardown armed event={kind} new_state={new_state}\n"
        ));
    }
    unsafe { gtk_main_do_event(event) };
    clear_gdk_dispatch();
}

static PREVIOUS_X_ERROR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "C" fn on_x_error(display: *mut std::ffi::c_void, event: *mut XErrorEvent) -> i32 {
    let previous = PREVIOUS_X_ERROR.load(std::sync::atomic::Ordering::SeqCst);
    let previous = if previous == 0 {
        None
    } else {
        Some(unsafe { std::mem::transmute::<usize, XErrorHandler>(previous) })
    };
    unsafe { dispatch_x_error(display, event, previous) }
}

unsafe extern "C" fn on_delete_event_emission(
    _hint: *mut std::ffi::c_void,
    _n_params: u32,
    params: *const std::ffi::c_void,
    _data: *mut std::ffi::c_void,
) -> i32 {
    let tracked_widget = TRACKED_GTK_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
    let emitting_widget = if !params.is_null() {
        *(params as *const *mut std::ffi::c_void)
    } else {
        std::ptr::null_mut()
    };
    if delete_event_targets_tracked(tracked_widget, emitting_widget) {
        note_window_teardown();
        write_stderr("node teardown armed via delete-event\n");
    }
    1
}

pub fn install_x_error_guard() {
    const _: () = assert!(std::mem::offset_of!(GdkEventWindowState, changed_mask) == 20);
    const _: () = assert!(std::mem::offset_of!(GdkEventWindowState, new_window_state) == 24);
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let previous = XSetErrorHandler(Some(on_x_error));
        PREVIOUS_X_ERROR.store(
            previous.map(|handler| handler as usize).unwrap_or(0),
            std::sync::atomic::Ordering::SeqCst,
        );
        gdk_event_handler_set(Some(on_gdk_event), std::ptr::null_mut(), None);
        let name = std::ffi::CString::new("delete-event").unwrap();
        let signal_id = g_signal_lookup(name.as_ptr(), gtk_widget_get_type());
        if signal_id != 0 {
            g_signal_add_emission_hook(
                signal_id,
                0,
                on_delete_event_emission,
                std::ptr::null_mut(),
                None,
            );
        }
    });
}

#[allow(dead_code)]
pub fn uninstall_x_error_guard() {
    let previous = PREVIOUS_X_ERROR.swap(0, std::sync::atomic::Ordering::SeqCst);
    if previous != 0 {
        let previous: XErrorHandler = unsafe { std::mem::transmute(previous) };
        unsafe {
            XSetErrorHandler(Some(previous));
        }
    }
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
    let msg = "node close requested\n";
    unsafe {
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
    let decision = state.close_window();
    match &decision {
        WindowClose::AllowExit => {
            note_window_teardown();
            let msg = "node close approved; teardown armed\n";
            unsafe {
                libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
            }
        }
        WindowClose::StayOpen { error } => {
            disarm_window_teardown();
            eprintln!("node stop was not confirmed; keeping the window open: {error}");
        }
    }
    decision
}

fn main() {
    if let Some(code) = run_parent_death_reaper_from_args(std::env::args().skip(1)) {
        std::process::exit(code);
    }
    enable_parent_death_reaper();
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
        .setup(|app| {
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(gtk_win) = win.gtk_window() {
                    let ptr = &gtk_win as *const _ as *const *mut std::ffi::c_void;
                    let raw_widget = unsafe { *ptr };
                    TRACKED_GTK_WINDOW.store(raw_widget, std::sync::atomic::Ordering::SeqCst);
                    let gdk_win = unsafe { gtk_widget_get_window(raw_widget) };
                    if !gdk_win.is_null() {
                        TRACKED_GDK_WINDOW.store(gdk_win, std::sync::atomic::Ordering::SeqCst);
                        let xid = unsafe { gdk_x11_window_get_xid(gdk_win) };
                        set_tracked_window_xid(xid);
                    }
                }
            }
            Ok(())
        })
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
