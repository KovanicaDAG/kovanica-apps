#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
/// Kovanica Node desktop application.
//
// Supervises the real `kovanica-node` binary as a Tauri sidecar and exposes
// the IPC contract documented in `apps/node-desktop/IPC-CONTRACT.md`.
use anyhow::Context;
use kovanica_crypto;
use kovanica_keystore;
use kovanica_types::crypto::Signature;
use kovanica_types::signing::SigningContext;
use kovanica_types::transaction::{Transaction, TransactionKind};
use kovanica_types::{self, Address, Hash, SigningKey};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use tauri::Manager;
use tauri::State;

// ============================================================================
// Constants
// ============================================================================

const EXPECTED_NETWORK_TAG: &str = "4-0eacd5699210b145";
const CHAIN_ID: u32 = 4;
const MIN_STAKE_ATOMS: u64 = 50_000_000_000_000; // 50_000 KUNA, 9 decimals
const MIN_STAKE_KUNA: &str = "50000";
const RPC_URL: &str = "http://127.0.0.1:8545/rpc";
const LOG_LINES: usize = 200;
const DEFAULT_CONFIG: &str = r#"chain_id = 4
data_dir = "<DATA_DIR>"
listen_addr = "0.0.0.0:8000"
rpc_addr = "127.0.0.1"
rpc_port = 8545
run_validator = false
round_duration_ms = 4000
bootnodes = ["seed1.kovanica.online:8000", "seed2.kovanica.online:8000", "seed3.kovanica.online:8000"]
"#;

// ============================================================================
// DTOs
// ============================================================================

#[derive(Clone, Serialize)]
struct StatusDto {
    state: String,
    pid: Option<u32>,
    network_tag: Option<String>,
    expected_tag: String,
    chain_id: u32,
    committed_height: Option<u64>,
    local_round: Option<u64>,
    peer_count: Option<u32>,
    catching_up: Option<bool>,
    rpc_url: String,
    data_dir: String,
    config_path: String,
    last_error: Option<String>,
}

#[derive(Clone, Serialize)]
struct ValidatorDto {
    run_validator: bool,
    has_key: bool,
    key_address: Option<String>,
    stake_atoms: Option<String>,
    stake_kuna: Option<String>,
    min_stake_atoms: String,
    min_stake_kuna: String,
    eligible: bool,
    can_toggle: bool,
    stake_available: bool,
    message: Option<String>,
}

#[derive(Clone, Serialize)]
struct KeyDto {
    exists: bool,
    path: Option<String>,
    address: Option<String>,
}

#[derive(Clone, Serialize)]
struct ActionResult {
    ok: bool,
    message: Option<String>,
    tx_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct SettingsDto {
    keep_running: bool,
}

// ============================================================================
// App state
// ============================================================================

struct NodeApp {
    sidecar: Arc<Mutex<Option<tokio::process::Child>>>,
    config_path: Arc<Mutex<Option<PathBuf>>>,
    data_dir: Arc<Mutex<Option<PathBuf>>>,
    keep_running: Arc<AtomicBool>,
    validator_enabled: Arc<AtomicBool>,
    logs: Arc<Mutex<Vec<String>>>,
    last_error: Arc<Mutex<Option<String>>>,
    key_path: Arc<Mutex<Option<PathBuf>>>,
    cached_signing_key: Arc<Mutex<Option<SigningKey>>>,
    cached_address: Arc<Mutex<Option<Address>>>,
    settings_path: Arc<Mutex<Option<PathBuf>>>,
}

// ============================================================================
// Helpers
// ============================================================================

fn node_binary() -> String {
    // Prefer a sidecar next to the current executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Try target-triple named sidecar first.
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("kovanica-node") && !name.ends_with(".d") {
                    return entry.path().to_string_lossy().to_string();
                }
            }
        }
    }
    // Fallback to PATH.
    "kovanica-node".to_string()
}

async fn rpc_call(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .map_err(|e| format!("rpc send failed: {e}"))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("rpc parse failed: {e}"))?;
    if let Some(err) = body.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    Ok(body
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

fn parse_quantity_value(v: &serde_json::Value) -> Option<u64> {
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        let s = s.strip_prefix("0x").unwrap_or(s);
        return u64::from_str_radix(s, 16).ok();
    }
    None
}

fn atom_to_kuna_str(atoms: u64) -> String {
    // 9 decimals: atoms / 1e9
    let whole = atoms / 1_000_000_000;
    let frac = atoms % 1_000_000_000;
    format!("{whole}.{frac:09}")
}

fn keystore_path_for(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create data dir: {e}"))?;
    Ok(dir.join("keystore.json"))
}

fn settings_path_for(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create data dir: {e}"))?;
    Ok(dir.join("settings.json"))
}

fn read_settings(path: &PathBuf) -> SettingsDto {
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let kr = v
                .get("keep_running")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            return SettingsDto { keep_running: kr };
        }
    }
    SettingsDto {
        keep_running: false,
    }
}

fn write_settings(path: &PathBuf, s: &SettingsDto) -> Result<(), String> {
    let v = serde_json::json!({ "keep_running": s.keep_running });
    std::fs::write(path, v.to_string()).map_err(|e| format!("write settings: {e}"))
}

fn address_to_string(a: &Address) -> String {
    a.to_string()
}

async fn ensure_config(app: &tauri::AppHandle, state: &NodeApp) -> Result<PathBuf, String> {
    // If already set, return it.
    {
        let guard = state.config_path.lock().await;
        if let Some(p) = guard.clone() {
            return Ok(p);
        }
    }
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let data_dir = data_dir.join("data");
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create data dir: {e}"))?;

    // Copy genesis files from bundle if data dir is empty (never overwrite).
    // Try a few candidate locations.
    let genesis_candidates = [
        app.path()
            .resource_dir()
            .ok()
            .map(|p| p.join("testnet").join("chain4")),
        Some(PathBuf::from("testnet/chain4")),
        Some(PathBuf::from("../testnet/chain4")),
    ];
    for cand in genesis_candidates.into_iter().flatten() {
        if cand.exists() {
            if let Ok(entries) = std::fs::read_dir(&cand) {
                for entry in entries.flatten() {
                    let dest = data_dir.join(entry.file_name());
                    if !dest.exists() {
                        let _ = std::fs::copy(entry.path(), &dest);
                    }
                }
            }
            break;
        }
    }

    let config_path = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("kovanica.toml");
    let content = DEFAULT_CONFIG.replace("<DATA_DIR>", data_dir.to_str().unwrap_or("."));
    std::fs::write(&config_path, &content).map_err(|e| format!("write config: {e}"))?;

    {
        let mut g = state.data_dir.lock().await;
        *g = Some(data_dir);
    }
    {
        let mut g = state.config_path.lock().await;
        *g = Some(config_path.clone());
    }
    Ok(config_path)
}

// ============================================================================
// Commands
// ============================================================================

#[tauri::command]
async fn node_status(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
) -> Result<StatusDto, String> {
    let sidecar = state.sidecar.lock().await;
    let config_path = state.config_path.lock().await.clone();
    let data_dir = state.data_dir.lock().await.clone();
    let last_error = state.last_error.lock().await.clone();

    let (state_str, pid) = if let Some(child) = sidecar.as_ref() {
        ("running".to_string(), child.id())
    } else {
        // Could be starting/stopping; default stopped
        ("stopped".to_string(), None)
    };
    drop(sidecar);

    let cfg_str = config_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let data_str = data_dir
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // If running, fetch sync status from RPC
    let (committed_height, local_round, peer_count, catching_up, network_tag) =
        if state_str == "running" {
            match rpc_call(RPC_URL, "kovanica_syncStatus", serde_json::json!([])).await {
                Ok(v) => {
                    let ch = v
                        .get("committedHeight")
                        .and_then(|x| x.as_u64())
                        .or_else(|| v.get("committed_height").and_then(|x| x.as_u64()));
                    let lr = v
                        .get("localTipRound")
                        .and_then(|x| x.as_u64())
                        .or_else(|| v.get("local_tip_round").and_then(|x| x.as_u64()));
                    let pc = v
                        .get("peerCount")
                        .and_then(|x| x.as_u64())
                        .or_else(|| v.get("peer_count").and_then(|x| x.as_u64()))
                        .map(|x| x as u32);
                    let cu = v
                        .get("catchingUp")
                        .and_then(|x| x.as_bool())
                        .or_else(|| v.get("catching_up").and_then(|x| x.as_bool()));
                    let nt = v
                        .get("networkTag")
                        .and_then(|x| x.as_str())
                        .or_else(|| v.get("network_tag").and_then(|x| x.as_str()))
                        .map(|s| s.to_string());
                    (ch, lr, pc, cu, nt)
                }
                Err(_) => (None, None, None, None, None),
            }
        } else {
            (None, None, None, None, None)
        };

    Ok(StatusDto {
        state: state_str,
        pid,
        network_tag,
        expected_tag: EXPECTED_NETWORK_TAG.to_string(),
        chain_id: CHAIN_ID,
        committed_height,
        local_round,
        peer_count,
        catching_up,
        rpc_url: RPC_URL.to_string(),
        data_dir: data_str,
        config_path: cfg_str,
        last_error,
    })
}

#[tauri::command]
async fn node_start(app: tauri::AppHandle, state: State<'_, NodeApp>) -> Result<StatusDto, String> {
    {
        let sidecar = state.sidecar.lock().await;
        if sidecar.is_some() {
            drop(sidecar);
            return node_status(app, state).await;
        }
    }

    let config_path = ensure_config(&app, &*state).await?;

    // --check-config
    let bin = node_binary();
    let check = tokio::process::Command::new(&bin)
        .arg("--check-config")
        .arg("--config")
        .arg(&config_path)
        .output()
        .await
        .map_err(|e| format!("failed to run --check-config: {e}"))?;
    if !check.status.success() {
        let stderr = String::from_utf8_lossy(&check.stderr).to_string();
        let mut le = state.last_error.lock().await;
        *le = Some(format!("Config validation failed: {stderr}"));
        return Err(format!("Config validation failed: {stderr}"));
    }

    // network-tag
    let tag_out = tokio::process::Command::new(&bin)
        .arg("network-tag")
        .arg("--config")
        .arg(&config_path)
        .output()
        .await
        .map_err(|e| format!("failed to run network-tag: {e}"))?;
    let tag = String::from_utf8_lossy(&tag_out.stdout).trim().to_string();
    if tag != EXPECTED_NETWORK_TAG {
        let msg = format!(
            "Network tag mismatch: got '{}', expected '{}'",
            tag, EXPECTED_NETWORK_TAG
        );
        let mut le = state.last_error.lock().await;
        *le = Some(msg.clone());
        return Err(msg);
    }

    // Spawn sidecar
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("--config").arg(&config_path);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc_setsid();
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        // nothing special; kill via taskkill later
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start node: {e}"))?;

    // Pipe stdout/stderr to logs
    let logs = Arc::clone(&state.logs);
    if let Some(stdout) = child.stdout.take() {
        let logs_clone = Arc::clone(&logs);
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() {
                    let mut l = logs_clone.lock().await;
                    l.push(trimmed);
                    let excess = l.len().saturating_sub(LOG_LINES);
                    if excess > 0 {
                        l.drain(..excess);
                    }
                }
                line.clear();
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let logs_clone = Arc::clone(&logs);
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() {
                    let mut l = logs_clone.lock().await;
                    l.push(trimmed);
                    let excess = l.len().saturating_sub(LOG_LINES);
                    if excess > 0 {
                        l.drain(..excess);
                    }
                }
                line.clear();
            }
        });
    }

    {
        let mut le = state.last_error.lock().await;
        *le = None;
    }
    {
        let mut s = state.sidecar.lock().await;
        *s = Some(child);
    }

    node_status(app, state).await
}

#[cfg(unix)]
fn libc_setsid() {
    // call setsid via unsafe libc. We avoid adding libc crate by using raw syscall.
    // setsid syscall number on x86_64 linux is 112.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        {
            std::arch::asm!("syscall", in("rax") 112usize, lateout("rax") _);
        }
    }
}

#[tauri::command]
async fn node_stop(state: State<'_, NodeApp>) -> Result<StatusDto, String> {
    let mut sidecar_guard = state.sidecar.lock().await;
    if let Some(mut child) = sidecar_guard.take() {
        // Kill process group on unix via setsid; child.kill kills the process.
        let pid = child.id();
        #[cfg(unix)]
        {
            if let Some(p) = pid {
                // kill process group
                unsafe {
                    libc_killpg(p as i32);
                }
            }
        }
        #[cfg(windows)]
        {
            if let Some(p) = pid {
                let _ = tokio::process::Command::new("taskkill")
                    .args(["/T", "/F", "/PID", &p.to_string()])
                    .output()
                    .await;
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
        println!("sidecar stopped");
    } else {
        // still return status
    }
    drop(sidecar_guard);

    // Wait for RPC port free (best effort, up to 5s)
    for _ in 0..50 {
        if tokio::net::TcpStream::connect("127.0.0.1:8545")
            .await
            .is_err()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Clear logs
    {
        let mut logs = state.logs.lock().await;
        logs.clear();
    }

    // Build a stopped status manually (node_status would read rpc, fail)
    let config_path = state.config_path.lock().await.clone();
    let data_dir = state.data_dir.lock().await.clone();
    let last_error = state.last_error.lock().await.clone();
    Ok(StatusDto {
        state: "stopped".to_string(),
        pid: None,
        network_tag: None,
        expected_tag: EXPECTED_NETWORK_TAG.to_string(),
        chain_id: CHAIN_ID,
        committed_height: None,
        local_round: None,
        peer_count: None,
        catching_up: None,
        rpc_url: RPC_URL.to_string(),
        data_dir: data_dir
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        config_path: config_path
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        last_error,
    })
}

#[cfg(unix)]
unsafe fn libc_killpg(pgid: i32) {
    // killpg syscall
    #[cfg(target_arch = "x86_64")]
    {
        // syscall kill (62) with negative pid => process group
        std::arch::asm!(
            "syscall",
            in("rax") 62usize,
            in("rdi") (-(pgid as i64)) as usize,
            in("rsi") 9usize, // SIGKILL
            lateout("rax") _,
        );
    }
}

#[tauri::command]
async fn node_logs(state: State<'_, NodeApp>) -> Result<Vec<String>, String> {
    let logs = state.logs.lock().await;
    Ok(logs.clone())
}

#[tauri::command]
async fn validator_state(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
) -> Result<ValidatorDto, String> {
    let key_path = {
        let g = state.key_path.lock().await;
        g.clone()
            .unwrap_or_else(|| keystore_path_for(&app).unwrap_or_default())
    };
    // ensure key_path stored
    {
        let mut g = state.key_path.lock().await;
        if g.is_none() {
            *g = Some(key_path.clone());
        }
    }

    let has_key = key_path.exists();
    let key_address = if has_key {
        match kovanica_keystore::load(&key_path) {
            Ok(ks) => kovanica_keystore::address(&ks).ok().map(|a| a.to_string()),
            Err(_) => None,
        }
    } else {
        None
    };

    // run_validator from config
    let run_validator = read_run_validator(&*state).await;

    // stake over RPC
    let (stake_atoms, catching_up) = if let Some(addr) = &key_address {
        let stake_res = rpc_call(RPC_URL, "kovanica_getStake", serde_json::json!([addr])).await;
        let stake = stake_res.ok().and_then(|v| parse_quantity_value(&v));
        let sync = rpc_call(RPC_URL, "kovanica_syncStatus", serde_json::json!([]))
            .await
            .ok();
        let cu = sync.as_ref().and_then(|v| {
            v.get("catchingUp")
                .and_then(|x| x.as_bool())
                .or_else(|| v.get("catching_up").and_then(|x| x.as_bool()))
        });
        (stake, cu)
    } else {
        (None, None)
    };

    let eligible = stake_atoms.map(|s| s >= MIN_STAKE_ATOMS).unwrap_or(false);
    let stake_available = state_running(&*state).await && !catching_up.unwrap_or(false);
    let can_toggle = state_running(&*state).await && eligible && !catching_up.unwrap_or(false);

    let message = if !has_key {
        Some("No validator key. Create or import one first.".to_string())
    } else if !eligible {
        Some(format!("Stake below minimum of {} KUNA.", MIN_STAKE_KUNA))
    } else if catching_up.unwrap_or(false) {
        Some("Node is catching up; staking unavailable.".to_string())
    } else {
        None
    };

    let stake_kuna = stake_atoms.map(|s| atom_to_kuna_str(s));

    Ok(ValidatorDto {
        run_validator,
        has_key,
        key_address,
        stake_atoms: stake_atoms.map(|s| s.to_string()),
        stake_kuna,
        min_stake_atoms: MIN_STAKE_ATOMS.to_string(),
        min_stake_kuna: MIN_STAKE_KUNA.to_string(),
        eligible,
        can_toggle,
        stake_available,
        message,
    })
}

async fn state_running(state: &NodeApp) -> bool {
    let g = state.sidecar.lock().await;
    g.is_some()
}

async fn read_run_validator(state: &NodeApp) -> bool {
    let cfg = state.config_path.lock().await.clone();
    if let Some(p) = cfg {
        if let Ok(text) = std::fs::read_to_string(&p) {
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("run_validator") {
                    return line.contains("true");
                }
            }
        }
    }
    false
}

#[tauri::command]
async fn set_run_validator(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
    enabled: bool,
) -> Result<ValidatorDto, String> {
    if enabled {
        // Check eligibility
        let vs = validator_state(app.clone(), state.clone()).await?;
        if !vs.eligible {
            return Err(vs.message.unwrap_or_else(|| "Not eligible.".to_string()));
        }
        if !vs.stake_available {
            return Err("Staking unavailable (node catching up or stopped).".to_string());
        }
        // write config
        write_run_validator(&*state, true).await?;
        state.validator_enabled.store(true, Ordering::Relaxed);
    } else {
        write_run_validator(&*state, false).await?;
        state.validator_enabled.store(false, Ordering::Relaxed);
    }
    // restart sidecar if running
    {
        let running = state.sidecar.lock().await.is_some();
        if running {
            drop(state.sidecar.lock().await);
            let _ = node_stop(state.clone()).await;
            let _ = node_start(app.clone(), state.clone()).await;
        }
    }
    validator_state(app, state).await
}

async fn write_run_validator(state: &NodeApp, val: bool) -> Result<(), String> {
    let cfg = state
        .config_path
        .lock()
        .await
        .clone()
        .ok_or("no config path")?;
    let text = std::fs::read_to_string(&cfg).map_err(|e| format!("read config: {e}"))?;
    let new_text =
        if text.contains("run_validator = true") || text.contains("run_validator = false") {
            text.replace(
                "run_validator = true",
                if val {
                    "run_validator = true"
                } else {
                    "run_validator = false"
                },
            )
            .replace(
                "run_validator = false",
                if val {
                    "run_validator = true"
                } else {
                    "run_validator = false"
                },
            )
        } else {
            format!("{text}\nrun_validator = {}", val)
        };
    // The above double-replace is wrong; redo cleanly:
    let new_text = if val {
        text.replace("run_validator = false", "run_validator = true")
    } else {
        text.replace("run_validator = true", "run_validator = false")
    };
    let new_text = if new_text == text && !text.contains("run_validator") {
        format!("{text}\nrun_validator = {}", val)
    } else {
        new_text
    };
    std::fs::write(&cfg, new_text).map_err(|e| format!("write config: {e}"))
}

#[tauri::command]
async fn stake(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
    amount_kuna: String,
) -> Result<ActionResult, String> {
    // Parse amount (KUNA decimal -> atoms). Accept atoms? Contract says amount_kuna.
    // Convert "50000" (KUNA) to atoms.
    let amount_atoms = parse_kuna_to_atoms(&amount_kuna)?;
    if amount_atoms == 0 {
        return Ok(ActionResult {
            ok: false,
            message: Some("Amount must be > 0.".into()),
            tx_id: None,
        });
    }

    // Need cached signing key
    let signing_key = {
        let g = state.cached_signing_key.lock().await;
        g.clone()
    };
    let sender = {
        let g = state.cached_address.lock().await;
        g.clone()
    };
    let (signing_key, sender) = match (signing_key, sender) {
        (Some(sk), Some(addr)) => (sk, addr),
        _ => {
            // Try loading from keystore with empty passphrase? No. Return not ready.
            return Ok(ActionResult {
                ok: false,
                message: Some("No unlocked key. Create or import a key first.".into()),
                tx_id: None,
            });
        }
    };

    let keystore_path = state
        .key_path
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| keystore_path_for(&app).unwrap());
    // verify keystore exists
    if !keystore_path.exists() {
        return Ok(ActionResult {
            ok: false,
            message: Some("No keystore file.".into()),
            tx_id: None,
        });
    }

    let running = state_running(&state).await;
    Ok(stake_flow(&HttpRpc, running, &signing_key, sender, amount_atoms).await)
}

// ============================================================================
// Chain guard + signing (stake / unbond)
// ============================================================================
//
// Every sign path goes through `ensure_chain` and `estimate_fee_rate` before a
// key touches a transaction. Any failure returns `ActionResult { ok: false }`
// with one of the refusal messages in IPC-CONTRACT.md ("Sign-path refusals"):
// nothing is signed and `kovanica_sendRawTransaction` is never called.

/// JSON-RPC transport for the sign paths. `HttpRpc` talks to the local node;
/// tests use a recording mock.
trait Rpc {
    fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value, String>> + Send;
}

struct HttpRpc;

impl Rpc for HttpRpc {
    fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value, String>> + Send {
        let method = method.to_string();
        async move { rpc_call(RPC_URL, &method, params).await }
    }
}

fn refuse(message: String) -> ActionResult {
    ActionResult {
        ok: false,
        message: Some(message),
        tx_id: None,
    }
}

/// Chain id reported by a `kovanica_syncStatus` object: numeric `chainId` /
/// `chain_id` when present, else the `<chain_id>-` prefix of `networkTag`.
fn reported_chain_id(sync: &serde_json::Value, tag: &str) -> Option<u64> {
    for key in ["chainId", "chain_id"] {
        if let Some(v) = sync.get(key) {
            return parse_quantity_value(v);
        }
    }
    tag.split_once('-')
        .and_then(|(id, _)| id.parse::<u64>().ok())
}

/// Refuse to sign unless the node is running, `kovanica_syncStatus` reports
/// chain id `CHAIN_ID`, and its `networkTag` equals `EXPECTED_NETWORK_TAG`.
async fn ensure_chain<R: Rpc>(rpc: &R, running: bool) -> Result<(), String> {
    if !running {
        return Err("Node is not running. Start the node before signing.".to_string());
    }
    let sync = rpc
        .call("kovanica_syncStatus", serde_json::json!([]))
        .await
        .map_err(|e| {
            format!("Cannot verify chain: kovanica_syncStatus failed ({e}). Refusing to sign.")
        })?;
    let tag = sync
        .get("networkTag")
        .or_else(|| sync.get("network_tag"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            "Cannot verify chain: node did not report a network tag. Refusing to sign.".to_string()
        })?;
    match reported_chain_id(&sync, tag) {
        Some(id) if id == CHAIN_ID as u64 => {}
        Some(id) => {
            return Err(format!(
                "Chain id mismatch: node reports {id}, expected {CHAIN_ID}. Refusing to sign."
            ));
        }
        None => {
            return Err(
                "Cannot verify chain: node did not report a chain id. Refusing to sign."
                    .to_string(),
            );
        }
    }
    if tag != EXPECTED_NETWORK_TAG {
        return Err(format!(
            "Network tag mismatch: node reports '{tag}', expected '{EXPECTED_NETWORK_TAG}'. Refusing to sign."
        ));
    }
    Ok(())
}

/// Fee rate (atoms per byte) from `kovanica_estimateFee`. No fallback.
async fn estimate_fee_rate<R: Rpc>(rpc: &R) -> Result<u64, String> {
    let v = rpc
        .call("kovanica_estimateFee", serde_json::json!([]))
        .await
        .map_err(|e| {
            format!(
                "Fee estimate unavailable: kovanica_estimateFee failed ({e}). Refusing to sign."
            )
        })?;
    match parse_quantity_value(&v) {
        Some(rate) if rate >= 1 => Ok(rate),
        _ => Err(format!(
            "Fee estimate invalid: kovanica_estimateFee returned {v}. Refusing to sign."
        )),
    }
}

/// Fee in atoms: `rate` x encoded size, the same rule as `packages/signer`
/// `feeForRate` and the light FFI (bincode size does not depend on the fee).
fn fee_for_rate(
    rate: u64,
    sender: Address,
    nonce: u64,
    kind: &TransactionKind,
) -> Result<u64, String> {
    let draft = Transaction {
        sender,
        nonce,
        kind: kind.clone(),
        fee: 1,
        signature: Signature([0u8; 64]),
        hash: Hash::zero(),
    };
    let size = bincode::serialize(&draft)
        .map_err(|e| format!("encode: {e}"))?
        .len() as u64;
    Ok(rate.saturating_mul(size).max(1))
}

/// Nonce + RPC fee, then sign and submit. Callers must pass `ensure_chain` first.
async fn sign_and_submit<R: Rpc>(
    rpc: &R,
    signing_key: &SigningKey,
    sender: Address,
    kind: TransactionKind,
) -> ActionResult {
    let nonce_val = match rpc
        .call("kovanica_getNonce", serde_json::json!([sender.to_string()]))
        .await
    {
        Ok(v) => v,
        Err(e) => return refuse(e),
    };
    let nonce = match parse_quantity_value(&nonce_val) {
        Some(n) => n,
        None => return refuse("Bad nonce.".into()),
    };
    let rate = match estimate_fee_rate(rpc).await {
        Ok(r) => r,
        Err(e) => return refuse(e),
    };
    let fee = match fee_for_rate(rate, sender, nonce, &kind) {
        Ok(f) => f,
        Err(e) => return refuse(e),
    };

    let ctx = SigningContext::new(CHAIN_ID as u64);
    let mut tx = Transaction {
        sender,
        nonce,
        kind,
        fee,
        signature: Signature([0u8; 64]),
        hash: Hash::zero(),
    };
    let hash = tx.signing_hash(&ctx);
    tx.signature = kovanica_crypto::sign(signing_key, hash.as_ref());
    tx.hash = hash;
    let raw = match bincode::serialize(&tx) {
        Ok(b) => hex::encode(b),
        Err(e) => return refuse(format!("encode: {e}")),
    };

    match rpc
        .call("kovanica_sendRawTransaction", serde_json::json!([raw]))
        .await
    {
        Ok(v) => {
            let tx_id = v.as_str().map(|s| s.to_string());
            ActionResult {
                ok: true,
                message: Some("Submitted.".into()),
                tx_id,
            }
        }
        Err(e) => refuse(e),
    }
}

async fn stake_flow<R: Rpc>(
    rpc: &R,
    running: bool,
    signing_key: &SigningKey,
    sender: Address,
    amount_atoms: u64,
) -> ActionResult {
    if let Err(e) = ensure_chain(rpc, running).await {
        return refuse(e);
    }
    sign_and_submit(
        rpc,
        signing_key,
        sender,
        TransactionKind::Stake {
            amount: amount_atoms,
        },
    )
    .await
}

async fn unbond_flow<R: Rpc>(
    rpc: &R,
    running: bool,
    signing_key: &SigningKey,
    sender: Address,
) -> ActionResult {
    if let Err(e) = ensure_chain(rpc, running).await {
        return refuse(e);
    }
    // Unbond the current stake in full.
    let stake_res = rpc
        .call("kovanica_getStake", serde_json::json!([sender.to_string()]))
        .await;
    let amount_atoms = stake_res
        .ok()
        .and_then(|v| parse_quantity_value(&v))
        .unwrap_or(0);
    if amount_atoms == 0 {
        return refuse("No stake to unbond.".into());
    }
    sign_and_submit(
        rpc,
        signing_key,
        sender,
        TransactionKind::Unstake {
            amount: amount_atoms,
        },
    )
    .await
}

fn parse_kuna_to_atoms(s: &str) -> Result<u64, String> {
    // Accept decimal KUNA like "50000" or "50000.0" -> atoms (9 decimals). Also "50000KUNA".
    let s = s.trim().to_uppercase();
    let s = s.replace("KUNA", "");
    let s = s.trim();
    if s.contains('.') {
        // parse as f64-ish manually
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 2 {
            return Err("bad amount".to_string());
        }
        let whole: u64 = parts[0].parse().map_err(|_| "bad amount".to_string())?;
        let frac_str = parts[1];
        let mut frac = frac_str.to_string();
        while frac.len() < 9 {
            frac.push('0');
        }
        if frac.len() > 9 {
            frac.truncate(9);
        }
        let frac: u64 = frac.parse().map_err(|_| "bad amount".to_string())?;
        Ok(whole * 1_000_000_000 + frac)
    } else {
        let n: u64 = s.parse().map_err(|_| "bad amount".to_string())?;
        Ok(n * 1_000_000_000)
    }
}

#[tauri::command]
async fn unbond(app: tauri::AppHandle, state: State<'_, NodeApp>) -> Result<ActionResult, String> {
    let signing_key = {
        let g = state.cached_signing_key.lock().await;
        g.clone()
    };
    let sender = {
        let g = state.cached_address.lock().await;
        g.clone()
    };
    let (signing_key, sender) = match (signing_key, sender) {
        (Some(sk), Some(addr)) => (sk, addr),
        _ => {
            return Ok(ActionResult {
                ok: false,
                message: Some("No unlocked key. Create or import a key first.".into()),
                tx_id: None,
            })
        }
    };

    let _ = app;
    let running = state_running(&state).await;
    Ok(unbond_flow(&HttpRpc, running, &signing_key, sender).await)
}

#[tauri::command]
async fn key_status(app: tauri::AppHandle, state: State<'_, NodeApp>) -> Result<KeyDto, String> {
    let path = {
        let g = state.key_path.lock().await;
        g.clone()
            .unwrap_or_else(|| keystore_path_for(&app).unwrap_or_default())
    };
    {
        let mut g = state.key_path.lock().await;
        if g.is_none() {
            *g = Some(path.clone());
        }
    }
    if !path.exists() {
        return Ok(KeyDto {
            exists: false,
            path: None,
            address: None,
        });
    }
    let addr = kovanica_keystore::load(&path)
        .ok()
        .and_then(|ks| kovanica_keystore::address(&ks).ok().map(|a| a.to_string()));
    Ok(KeyDto {
        exists: true,
        path: Some(path.to_string_lossy().to_string()),
        address: addr,
    })
}

#[tauri::command]
async fn key_create(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
    passphrase: String,
) -> Result<KeyDto, String> {
    let path = keystore_path_for(&app)?;
    let ks = kovanica_keystore::keygen(&passphrase).map_err(|e| format!("keygen: {e}"))?;
    kovanica_keystore::save(&path, &ks).map_err(|e| format!("save: {e}"))?;
    // tighten perms
    // cache signing key
    let sk = kovanica_keystore::signing_key(&ks, Some(&passphrase))
        .map_err(|e| format!("signing_key: {e}"))?;
    let addr = kovanica_keystore::address(&ks).map_err(|e| format!("address: {e}"))?;
    {
        let mut g = state.cached_signing_key.lock().await;
        *g = Some(sk);
    }
    {
        let mut g = state.cached_address.lock().await;
        *g = Some(addr);
    }
    {
        let mut g = state.key_path.lock().await;
        *g = Some(path.clone());
    }
    Ok(KeyDto {
        exists: true,
        path: Some(path.to_string_lossy().to_string()),
        address: Some(addr.to_string()),
    })
}

#[tauri::command]
async fn key_import(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
    secret: String,
    passphrase: String,
) -> Result<KeyDto, String> {
    let path = keystore_path_for(&app)?;
    // secret is hex seed (32 bytes) or mnemonic? Contract says "secret". Try hex seed first.
    let seed = if secret.split_whitespace().count() >= 12 {
        kovanica_keystore::seed_from_mnemonic(&secret).map_err(|e| format!("mnemonic: {e}"))?
    } else {
        kovanica_keystore::seed_from_raw_hex(&secret).map_err(|e| format!("seed: {e}"))?
    };
    let ks =
        kovanica_keystore::from_seed(&seed, &passphrase).map_err(|e| format!("from_seed: {e}"))?;
    kovanica_keystore::save(&path, &ks).map_err(|e| format!("save: {e}"))?;
    let sk = kovanica_keystore::signing_key(&ks, Some(&passphrase))
        .map_err(|e| format!("signing_key: {e}"))?;
    let addr = kovanica_keystore::address(&ks).map_err(|e| format!("address: {e}"))?;
    {
        let mut g = state.cached_signing_key.lock().await;
        *g = Some(sk);
    }
    {
        let mut g = state.cached_address.lock().await;
        *g = Some(addr);
    }
    {
        let mut g = state.key_path.lock().await;
        *g = Some(path.clone());
    }
    Ok(KeyDto {
        exists: true,
        path: Some(path.to_string_lossy().to_string()),
        address: Some(addr.to_string()),
    })
}

#[tauri::command]
async fn settings_get(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
) -> Result<SettingsDto, String> {
    let path = {
        let g = state.settings_path.lock().await;
        g.clone()
            .unwrap_or_else(|| settings_path_for(&app).unwrap_or_default())
    };
    {
        let mut g = state.settings_path.lock().await;
        if g.is_none() {
            *g = Some(path.clone());
        }
    }
    Ok(read_settings(&path))
}

#[tauri::command]
async fn set_keep_running(
    app: tauri::AppHandle,
    state: State<'_, NodeApp>,
    enabled: bool,
) -> Result<SettingsDto, String> {
    let path = {
        let g = state.settings_path.lock().await;
        g.clone()
            .unwrap_or_else(|| settings_path_for(&app).unwrap_or_default())
    };
    {
        let mut g = state.settings_path.lock().await;
        if g.is_none() {
            *g = Some(path.clone());
        }
    }
    let s = SettingsDto {
        keep_running: enabled,
    };
    write_settings(&path, &s)?;
    state.keep_running.store(enabled, Ordering::Relaxed);
    Ok(s)
}

// ============================================================================
// Setup
// ============================================================================

fn install(_app: &tauri::AppHandle) -> NodeApp {
    NodeApp {
        sidecar: Arc::new(Mutex::new(None)),
        config_path: Arc::new(Mutex::new(None)),
        data_dir: Arc::new(Mutex::new(None)),
        keep_running: Arc::new(AtomicBool::new(false)),
        validator_enabled: Arc::new(AtomicBool::new(false)),
        logs: Arc::new(Mutex::new(Vec::new())),
        last_error: Arc::new(Mutex::new(None)),
        key_path: Arc::new(Mutex::new(None)),
        cached_signing_key: Arc::new(Mutex::new(None)),
        cached_address: Arc::new(Mutex::new(None)),
        settings_path: Arc::new(Mutex::new(None)),
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(install(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            node_status,
            node_start,
            node_stop,
            node_logs,
            validator_state,
            set_run_validator,
            stake,
            unbond,
            key_status,
            key_create,
            key_import,
            settings_get,
            set_keep_running,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kovanica Node");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kuna_to_atoms_works() {
        assert_eq!(parse_kuna_to_atoms("50000").unwrap(), 50000 * 1_000_000_000);
        assert_eq!(parse_kuna_to_atoms("1.5").unwrap(), 1_500_000_000);
    }

    #[test]
    fn min_stake_constant() {
        assert_eq!(MIN_STAKE_ATOMS, 50_000_000_000_000);
    }

    // ---- chain guard / fee (sign paths) ------------------------------------

    /// Recording JSON-RPC mock: canned result per method, every call logged.
    struct MockRpc {
        responses: std::collections::HashMap<&'static str, Result<serde_json::Value, String>>,
        calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl MockRpc {
        fn new(responses: Vec<(&'static str, Result<serde_json::Value, String>)>) -> Self {
            MockRpc {
                responses: responses.into_iter().collect(),
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        /// Healthy chain-4 node with the given network tag and fee rate.
        fn chain(tag: &str, fee: Result<serde_json::Value, String>) -> Self {
            Self::new(vec![
                (
                    "kovanica_syncStatus",
                    Ok(serde_json::json!({ "networkTag": tag, "catchingUp": false })),
                ),
                ("kovanica_getNonce", Ok(serde_json::json!("0x7"))),
                ("kovanica_getStake", Ok(serde_json::json!("0x2d79883d2000"))),
                ("kovanica_estimateFee", fee),
                (
                    "kovanica_sendRawTransaction",
                    Ok(serde_json::json!("0xabc")),
                ),
            ])
        }

        fn methods(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(m, _)| m.clone())
                .collect()
        }

        fn sent_raw(&self) -> Option<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .find(|(m, _)| m == "kovanica_sendRawTransaction")
                .and_then(|(_, p)| p[0].as_str().map(|s| s.to_string()))
        }
    }

    impl Rpc for MockRpc {
        fn call(
            &self,
            method: &str,
            params: serde_json::Value,
        ) -> impl std::future::Future<Output = Result<serde_json::Value, String>> + Send {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            let out = self
                .responses
                .get(method)
                .cloned()
                .unwrap_or_else(|| Err(format!("unexpected method {method}")));
            async move { out }
        }
    }

    fn test_key() -> (SigningKey, Address) {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let addr = Address(sk.verifying_key().to_bytes());
        (sk, addr)
    }

    fn assert_refused_unsigned(r: &ActionResult, rpc: &MockRpc, needle: &str) {
        assert!(!r.ok, "must refuse");
        assert!(r.tx_id.is_none());
        let msg = r.message.clone().unwrap_or_default();
        assert!(
            msg.contains(needle),
            "message {msg:?} should contain {needle:?}"
        );
        let methods = rpc.methods();
        assert!(
            !methods.iter().any(|m| m == "kovanica_sendRawTransaction"),
            "sent: {methods:?}"
        );
        // Nonce/fee are read only after the guard passes; signing comes after both.
        assert!(
            !methods.iter().any(|m| m == "kovanica_getNonce"),
            "signed path reached: {methods:?}"
        );
        assert!(rpc.sent_raw().is_none());
    }

    #[tokio::test]
    async fn wrong_network_tag_refuses_stake_and_unbond_without_signing() {
        let (sk, addr) = test_key();
        let rpc = MockRpc::chain("4-0123456789abcdef", Ok(serde_json::json!("0x1")));
        let r = stake_flow(&rpc, true, &sk, addr, 50_000_000_000_000).await;
        assert_refused_unsigned(&r, &rpc, "Network tag mismatch");

        let rpc = MockRpc::chain("4-0123456789abcdef", Ok(serde_json::json!("0x1")));
        let r = unbond_flow(&rpc, true, &sk, addr).await;
        assert_refused_unsigned(&r, &rpc, "Network tag mismatch");
        assert!(!rpc.methods().iter().any(|m| m == "kovanica_getStake"));
    }

    #[tokio::test]
    async fn wrong_chain_id_refuses() {
        let (sk, addr) = test_key();
        let rpc = MockRpc::chain("5-0eacd5699210b145", Ok(serde_json::json!("0x1")));
        let r = stake_flow(&rpc, true, &sk, addr, 1).await;
        assert_refused_unsigned(&r, &rpc, "Chain id mismatch");

        let rpc = MockRpc::new(vec![(
            "kovanica_syncStatus",
            Ok(serde_json::json!({ "networkTag": EXPECTED_NETWORK_TAG, "chainId": 5 })),
        )]);
        let r = unbond_flow(&rpc, true, &sk, addr).await;
        assert_refused_unsigned(&r, &rpc, "Chain id mismatch");
    }

    #[tokio::test]
    async fn missing_tag_or_sync_failure_refuses() {
        let (sk, addr) = test_key();
        let rpc = MockRpc::new(vec![(
            "kovanica_syncStatus",
            Ok(serde_json::json!({ "catchingUp": false })),
        )]);
        let r = stake_flow(&rpc, true, &sk, addr, 1).await;
        assert_refused_unsigned(&r, &rpc, "did not report a network tag");

        let rpc = MockRpc::new(vec![(
            "kovanica_syncStatus",
            Err("rpc send failed: refused".into()),
        )]);
        let r = stake_flow(&rpc, true, &sk, addr, 1).await;
        assert_refused_unsigned(&r, &rpc, "kovanica_syncStatus failed");
    }

    #[tokio::test]
    async fn stopped_node_refuses_before_any_rpc() {
        let (sk, addr) = test_key();
        let rpc = MockRpc::chain(EXPECTED_NETWORK_TAG, Ok(serde_json::json!("0x1")));
        let r = stake_flow(&rpc, false, &sk, addr, 1).await;
        assert_refused_unsigned(&r, &rpc, "Node is not running");
        let r = unbond_flow(&rpc, false, &sk, addr).await;
        assert_refused_unsigned(&r, &rpc, "Node is not running");
        assert!(
            rpc.methods().is_empty(),
            "no RPC when stopped: {:?}",
            rpc.methods()
        );
    }

    #[tokio::test]
    async fn fee_estimate_failure_refuses_without_fallback() {
        let (sk, addr) = test_key();
        for fee in [
            Err("rpc error: boom".to_string()),
            Ok(serde_json::json!("0x0")),
            Ok(serde_json::Value::Null),
        ] {
            let rpc = MockRpc::chain(EXPECTED_NETWORK_TAG, fee);
            let r = stake_flow(&rpc, true, &sk, addr, 1).await;
            assert!(!r.ok);
            assert!(
                r.message.clone().unwrap().contains("Fee estimate"),
                "{:?}",
                r.message
            );
            assert!(
                rpc.sent_raw().is_none(),
                "must not submit with a fallback fee"
            );
        }
    }

    #[tokio::test]
    async fn fee_comes_from_rpc_rate_times_size() {
        let (sk, addr) = test_key();
        for (flow, rate) in [("stake", 3u64), ("unbond", 5u64)] {
            let rpc = MockRpc::chain(
                EXPECTED_NETWORK_TAG,
                Ok(serde_json::json!(format!("0x{rate:x}"))),
            );
            let r = if flow == "stake" {
                stake_flow(&rpc, true, &sk, addr, 50_000_000_000_000).await
            } else {
                unbond_flow(&rpc, true, &sk, addr).await
            };
            assert!(r.ok, "{flow}: {:?}", r.message);
            assert_eq!(r.tx_id.as_deref(), Some("0xabc"));
            let raw = hex::decode(rpc.sent_raw().expect("submitted")).unwrap();
            let tx: Transaction = bincode::deserialize(&raw).unwrap();
            assert_eq!(tx.nonce, 7);
            assert_eq!(tx.fee, rate * raw.len() as u64, "{flow}: fee = rate x size");
            assert_ne!(tx.fee, 1);
            assert_eq!(
                tx.hash,
                tx.signing_hash(&SigningContext::new(CHAIN_ID as u64))
            );
            let m = rpc.methods();
            assert_eq!(
                m.first().map(String::as_str),
                Some("kovanica_syncStatus"),
                "{flow}: guard first: {m:?}"
            );
            assert!(m.iter().any(|x| x == "kovanica_estimateFee"));
        }
    }
}
