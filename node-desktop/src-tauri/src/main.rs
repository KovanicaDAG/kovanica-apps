//! Kovanica Node milestone 1: Linux supervisor and read-only monitor.
//!
//! The webview cannot start a process except through these commands.
//! There is no remote admin endpoint and no transaction submission.

use std::time::Instant;

use kovanica_node_desktop::monitor::fetch_monitor;
use kovanica_node_desktop::policy::{check_setup, kovanica_env_names};
use kovanica_node_desktop::session::{BeginPoll, Session, Snapshot, WindowClose};
use kovanica_node_desktop::supervisor::{
    arm_exit_hook, disarm_window_teardown, gdk_event_starts_teardown_for_window,
    note_window_teardown, set_tracked_window_xid, should_suppress_x_error,
    stop_owned_on_process_exit, tracked_window_xid, window_teardown_started, ProcessState,
    XErrorEvent, XErrorHandler, STOP_GRACE,
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

/// `GdkEventType` of the event currently inside `gtk_main_do_event`.
static CURRENT_GDK_EVENT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static LAST_WINDOW_STATE_CHANGED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
static LAST_WINDOW_STATE_NEW: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static TRACKED_GTK_WINDOW: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static TRACKED_GDK_WINDOW: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static TRACKED_TOPLEVEL_XID: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

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

const GDK_WINDOW_STATE: i32 = 32;

unsafe extern "C" fn on_gdk_event(event: *mut std::ffi::c_void, _data: *mut std::ffi::c_void) {
    let kind = if event.is_null() {
        -1
    } else {
        unsafe { *(event as *const i32) }
    };
    CURRENT_GDK_EVENT.store(kind, std::sync::atomic::Ordering::SeqCst);

    let event_win = if event.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { (*(event as *const GdkEventAny)).window }
    };

    if !event_win.is_null()
        && TRACKED_GDK_WINDOW
            .load(std::sync::atomic::Ordering::SeqCst)
            .is_null()
        && unsafe { gdk_window_get_window_type(event_win) } == 1
    /* GDK_WINDOW_TOPLEVEL */
    {
        TRACKED_GDK_WINDOW.store(event_win, std::sync::atomic::Ordering::SeqCst);
        let xid = unsafe { gdk_x11_window_get_xid(event_win) };
        TRACKED_TOPLEVEL_XID.store(xid, std::sync::atomic::Ordering::SeqCst);
        set_tracked_window_xid(xid);
    }

    let tracked = TRACKED_GDK_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
    let is_tracked = !tracked.is_null() && event_win == tracked;

    let mut new_state = 0;
    if kind == GDK_WINDOW_STATE && !event.is_null() {
        let state = unsafe { &*(event as *const GdkEventWindowState) };
        new_state = state.new_window_state;
        LAST_WINDOW_STATE_CHANGED.store(state.changed_mask, std::sync::atomic::Ordering::SeqCst);
        LAST_WINDOW_STATE_NEW.store(new_state, std::sync::atomic::Ordering::SeqCst);
    }

    if gdk_event_starts_teardown_for_window(kind, new_state, is_tracked) {
        note_window_teardown();
        write_stderr(&format!(
            "node teardown armed event={kind} new_state={new_state}\n"
        ));
    }
    unsafe { gtk_main_do_event(event) };
}

static PREVIOUS_X_ERROR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "C" fn on_x_error(display: *mut std::ffi::c_void, event: *mut XErrorEvent) -> i32 {
    let code = unsafe { (*event).error_code };
    let resource_id = unsafe { (*event).resourceid };
    let teardown = window_teardown_started();
    let tracked_xid = tracked_window_xid();
    if should_suppress_x_error(code, resource_id, teardown, tracked_xid) {
        write_stderr(&format!("node x error {code} ignored during teardown\n"));
        return 0;
    }

    // Teardown race: WebKitGTK hits BadDrawable (9) or BadWindow (3) on the tracked
    // window drawable when the window manager unmaps/destroys it during close, before GTK
    // dispatches delete-event. Suppress only this specific race on the tracked window so the
    // clean shutdown path can run.
    if (code == 3 || code == 9) && tracked_xid != 0 && resource_id == tracked_xid {
        let gdk_event = CURRENT_GDK_EVENT.load(std::sync::atomic::Ordering::SeqCst);
        let changed = LAST_WINDOW_STATE_CHANGED.load(std::sync::atomic::Ordering::SeqCst);
        if gdk_event == GDK_WINDOW_STATE && (changed & 2) != 0 {
            note_window_teardown();
            write_stderr(&format!(
                "node x error {code} ignored during close unmap race on tracked window\n"
            ));
            return 0;
        }
    }

    if code == 3 || code == 9 {
        let request = unsafe { (*event).request_code };
        let gdk_event = CURRENT_GDK_EVENT.load(std::sync::atomic::Ordering::SeqCst);
        let changed = LAST_WINDOW_STATE_CHANGED.load(std::sync::atomic::Ordering::SeqCst);
        let new_state = LAST_WINDOW_STATE_NEW.load(std::sync::atomic::Ordering::SeqCst);
        write_stderr(&format!(
            "node x error {code} passed to toolkit teardown={teardown} request={request} gdk_event={gdk_event} window_changed={changed} window_new={new_state}\n"
        ));
    }
    let previous = PREVIOUS_X_ERROR.load(std::sync::atomic::Ordering::SeqCst);
    if previous != 0 {
        // GTK 3's error handler prints the error and calls _exit(1), skipping libc atexit.
        // Clean up the owned child process now so unexpected GTK exits do not leak the child.
        stop_owned_on_process_exit();
        let previous: XErrorHandler = unsafe { std::mem::transmute(previous) };
        return unsafe { previous(display, event) };
    }
    write_stderr(&format!("node x error {code} had no previous handler\n"));
    0
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
    if tracked_widget.is_null() || emitting_widget == tracked_widget {
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
                        TRACKED_TOPLEVEL_XID.store(xid, std::sync::atomic::Ordering::SeqCst);
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
