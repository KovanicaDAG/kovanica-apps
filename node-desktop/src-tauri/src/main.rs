//! Kovanica Node milestone 1: Linux supervisor and read-only monitor.
//!
//! The webview cannot start a process except through these commands.
//! There is no remote admin endpoint and no transaction submission.

use std::sync::Mutex;
use std::time::Instant;

use kovanica_node_desktop::monitor::{fetch_monitor, MonitorView, PollGate};
use kovanica_node_desktop::policy::{
    check_setup, kovanica_env_names, CheckedSetup, REVIEWED_NODE_SOURCE,
};
use kovanica_node_desktop::supervisor::{ProcessState, ProcessStatus, Supervisor, STOP_GRACE};
use serde::Serialize;
use tauri::Manager;
use tauri::State;

struct AppState {
    supervisor: Supervisor,
    monitor: Mutex<MonitorRuntime>,
    setup: Mutex<Option<CheckedSetup>>,
}

struct MonitorRuntime {
    gate: PollGate,
    rpc_http: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    process: ProcessStatus,
    monitor: MonitorView,
    setup: Option<CheckedSetup>,
    reviewed_node_source: &'static str,
    stop_grace_secs: u64,
    exit_stops_owned_child: bool,
}

fn current_kovanica_env() -> Vec<String> {
    let keys: Vec<String> = std::env::vars().map(|(key, _)| key).collect();
    let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    kovanica_env_names(refs)
}

fn stop_owned(state: &AppState) -> Result<(), String> {
    let result = state.supervisor.stop();
    let mut monitor = state.monitor.lock().expect("monitor lock");
    monitor.gate.bump_generation(Instant::now());
    monitor.rpc_http = None;
    result
}

fn snapshot(state: &AppState) -> Snapshot {
    let process = state.supervisor.status();
    let monitor = state
        .monitor
        .lock()
        .expect("monitor lock")
        .gate
        .last
        .clone();
    Snapshot {
        process,
        monitor,
        setup: state.setup.lock().expect("setup lock").clone(),
        reviewed_node_source: REVIEWED_NODE_SOURCE,
        stop_grace_secs: STOP_GRACE.as_secs(),
        exit_stops_owned_child: true,
    }
}

#[tauri::command]
fn inspect_setup(
    state: State<'_, AppState>,
    binary: String,
    config_path: String,
    data_dir: String,
) -> Result<CheckedSetup, String> {
    let checked = check_setup(
        std::path::Path::new(&binary),
        std::path::Path::new(&config_path),
        std::path::Path::new(&data_dir),
        &current_kovanica_env(),
    )?;
    *state.setup.lock().expect("setup lock") = Some(checked.clone());
    Ok(checked)
}

#[tauri::command]
fn node_start(
    state: State<'_, AppState>,
    binary: String,
    config_path: String,
    data_dir: String,
) -> Result<Snapshot, String> {
    let checked = check_setup(
        std::path::Path::new(&binary),
        std::path::Path::new(&config_path),
        std::path::Path::new(&data_dir),
        &current_kovanica_env(),
    )?;
    state.supervisor.start(
        std::path::Path::new(&checked.binary),
        std::path::Path::new(&checked.config_path),
    )?;
    {
        let mut monitor = state.monitor.lock().expect("monitor lock");
        monitor.gate.bump_generation(Instant::now());
        monitor.rpc_http = Some(checked.rpc_http.clone());
    }
    *state.setup.lock().expect("setup lock") = Some(checked);
    Ok(snapshot(&state))
}

#[tauri::command]
fn node_stop(state: State<'_, AppState>) -> Result<Snapshot, String> {
    stop_owned(&state)?;
    Ok(snapshot(&state))
}

#[tauri::command]
fn node_status(state: State<'_, AppState>) -> Snapshot {
    snapshot(&state)
}

#[tauri::command]
fn set_monitor_paused(state: State<'_, AppState>, paused: bool) {
    state.monitor.lock().expect("monitor lock").gate.paused = paused;
}

#[tauri::command]
async fn monitor_refresh(state: State<'_, AppState>) -> Result<Snapshot, String> {
    if state.supervisor.status().state != ProcessState::Running {
        return Ok(snapshot(&state));
    }
    let (generation, rpc_http) = {
        let mut monitor = state.monitor.lock().expect("monitor lock");
        let now = Instant::now();
        match monitor.gate.begin(now) {
            Some(generation) => (generation, monitor.rpc_http.clone()),
            None => return Ok(snapshot(&state)),
        }
    };
    let Some(rpc_http) = rpc_http else {
        let mut monitor = state.monitor.lock().expect("monitor lock");
        monitor
            .gate
            .finish_err(generation, Instant::now(), "no loopback endpoint".into());
        return Ok(snapshot(&state));
    };
    match fetch_monitor(&rpc_http).await {
        Ok(view) => {
            let mut monitor = state.monitor.lock().expect("monitor lock");
            monitor.gate.finish_ok(generation, Instant::now(), view);
        }
        Err(err) => {
            let mut monitor = state.monitor.lock().expect("monitor lock");
            monitor.gate.finish_err(generation, Instant::now(), err);
        }
    }
    Ok(snapshot(&state))
}

fn main() {
    let app = AppState {
        supervisor: Supervisor::new(STOP_GRACE),
        monitor: Mutex::new(MonitorRuntime {
            gate: PollGate::new(Instant::now()),
            rpc_http: None,
        }),
        setup: Mutex::new(None),
    };
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
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app.try_state::<AppState>() {
                    // Same path as the Stop button: only the child this process spawned.
                    let _ = stop_owned(state.inner());
                }
            }
        });
}
