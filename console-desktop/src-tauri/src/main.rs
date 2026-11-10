//! Kovanica Console desktop shell.
//!
//! The webview only displays what this process returns. RPC calls, chain-id
//! checks, and key handling stay on this side of the bridge.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod amount;
mod connection;
mod history;
mod transfer;
mod wallet;

use amount::{format_kuna_atoms, parse_kuna_atoms};

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use connection::{
    airdrop_hash, endpoint_label, faucet_chain_id, fetch_status, read_saved_url, resolve_rpc_url,
    write_saved_url, HttpRpc, NodeStatus, Rpc, RPC_URL_ENV,
};
use serde::Serialize;
use std::time::Instant;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use zeroize::Zeroizing;

/// Process-local wallet session. The keystore file lives in the app data directory.
///
/// `idle` is read so the lock thread stays alive for the process. Drop joins it.
struct WalletApp {
    idle: wallet::IdleLockThread,
    shared: Arc<wallet::WalletShared>,
    notify: Arc<dyn wallet::SessionNotify>,
}

/// Endpoint the screen should show and edit.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionConfig {
    rpc_url: String,
    endpoint_label: String,
}

fn endpoint_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "Could not open the app data directory.".to_string())?;
    std::fs::create_dir_all(&dir)
        .map_err(|_| "Could not open the app data directory.".to_string())?;
    Ok(dir.join("rpc-endpoint.json"))
}

fn current_url(app: &tauri::AppHandle) -> Result<String, String> {
    let saved = read_saved_url(&endpoint_path(app)?).map_err(|err| err.to_string())?;
    let env_value = env::var(RPC_URL_ENV).ok();
    Ok(resolve_rpc_url(saved.as_deref(), env_value.as_deref()))
}

fn config_for(rpc_url: String) -> ConnectionConfig {
    let endpoint_label = endpoint_label(&rpc_url);
    ConnectionConfig {
        rpc_url,
        endpoint_label,
    }
}

/// Saved endpoint, or the environment, or the public testnet DNS name.
#[tauri::command]
fn connection_config(app: tauri::AppHandle) -> Result<ConnectionConfig, String> {
    current_url(&app).map(config_for)
}

/// Store an endpoint in the app data directory and use it from now on.
#[tauri::command]
fn save_connection(app: tauri::AppHandle, rpc_url: String) -> Result<ConnectionConfig, String> {
    let saved = write_saved_url(&endpoint_path(&app)?, &rpc_url).map_err(|err| err.to_string())?;
    Ok(config_for(saved))
}

/// Forget the saved endpoint and return to the environment or the DNS default.
#[tauri::command]
fn reset_connection(app: tauri::AppHandle) -> Result<ConnectionConfig, String> {
    let path = endpoint_path(&app)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|_| "Could not reset the RPC setting.".to_string())?;
    }
    current_url(&app).map(config_for)
}

/// Parse a KUNA amount. The result is a decimal atom string.
#[tauri::command]
fn parse_kuna(amount: String) -> Result<String, String> {
    parse_kuna_atoms(&amount).map(|atoms| atoms.to_string())
}

/// Format an atom string as KUNA. The input is decimal, never a float.
#[tauri::command]
fn format_kuna(atoms: String) -> Result<String, String> {
    let atoms = parse_atom_string(&atoms)?;
    Ok(format_kuna_atoms(atoms))
}

fn wallet_file<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "Could not open the app data directory.".to_string())?;
    std::fs::create_dir_all(&dir)
        .map_err(|_| "Could not open the app data directory.".to_string())?;
    Ok(dir.join(wallet::KEYSTORE_FILE))
}

fn with_wallet<T>(
    state: &State<'_, WalletApp>,
    body: impl FnOnce(&mut wallet::WalletStore) -> Result<T, String>,
) -> Result<T, String> {
    let _idle = &state.idle;
    wallet::with_store(&state.shared, state.notify.as_ref(), body)
}

/// Whether a wallet file exists, and whether the session is open.
#[tauri::command]
fn wallet_status(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
) -> Result<wallet::WalletStatus, String> {
    let path = wallet_file(&app)?;
    with_wallet(&state, |store| store.status(&path, Instant::now()))
}

/// Create a key. The phrase is returned once.
#[tauri::command]
fn wallet_create(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    passphrase: String,
) -> Result<wallet::NewWallet, String> {
    let path = wallet_file(&app)?;
    let passphrase = Zeroizing::new(passphrase);
    with_wallet(&state, |store| {
        store.create(&path, passphrase.as_str(), Instant::now())
    })
}

/// Import a 24-word seed. The phrase is not returned.
#[tauri::command]
fn wallet_import(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    mnemonic: String,
    passphrase: String,
) -> Result<wallet::WalletStatus, String> {
    let path = wallet_file(&app)?;
    let mnemonic = Zeroizing::new(mnemonic);
    let passphrase = Zeroizing::new(passphrase);
    with_wallet(&state, |store| {
        store.import(
            &path,
            mnemonic.as_str(),
            passphrase.as_str(),
            Instant::now(),
        )
    })
}

/// Create an HD wallet with 12 or 24 words. The phrase is returned once.
#[tauri::command]
fn wallet_create_hd(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    word_count: u32,
    passphrase: String,
) -> Result<wallet::NewWallet, String> {
    let path = wallet_file(&app)?;
    let passphrase = Zeroizing::new(passphrase);
    with_wallet(&state, |store| {
        store.create_hd(&path, word_count, passphrase.as_str(), Instant::now())
    })
}

/// Import a 12- or 24-word BIP-39 mnemonic into an HD wallet.
#[tauri::command]
fn wallet_import_hd(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    mnemonic: String,
    passphrase: String,
) -> Result<wallet::WalletStatus, String> {
    let path = wallet_file(&app)?;
    let mnemonic = Zeroizing::new(mnemonic);
    let passphrase = Zeroizing::new(passphrase);
    with_wallet(&state, |store| {
        store.import_hd(
            &path,
            mnemonic.as_str(),
            passphrase.as_str(),
            Instant::now(),
        )
    })
}

/// Derive an account address at m/44'/3007'/<account_index>'/0'/0' from the unlocked HD session.
#[tauri::command]
fn wallet_derive_account(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    bip39_passphrase: String,
    account_index: u32,
) -> Result<String, String> {
    let path = wallet_file(&app)?;
    let bip39_passphrase = Zeroizing::new(bip39_passphrase);
    with_wallet(&state, |store| {
        store.derive_account(
            &path,
            bip39_passphrase.as_str(),
            account_index,
            Instant::now(),
        )
    })
}

/// Sign a reviewed HD transfer from the unlocked session.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn wallet_sign_transfer_hd(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    bip39_passphrase: String,
    account_index: u32,
    to_address: String,
    amount: String,
    fee: String,
    nonce: u64,
    chain_id: u64,
) -> Result<String, String> {
    let path = wallet_file(&app)?;
    let bip39_passphrase = Zeroizing::new(bip39_passphrase);
    let amount_atoms = parse_atom_string(&amount)?;
    let fee_atoms = parse_atom_string(&fee)?;
    with_wallet(&state, |store| {
        store.sign_transfer_hd(
            &path,
            bip39_passphrase.as_str(),
            account_index,
            &to_address,
            amount_atoms,
            fee_atoms,
            nonce,
            chain_id,
            Instant::now(),
        )
    })
}

fn parse_u64_json(value: &serde_json::Value, label: &str) -> Result<u64, String> {
    match value {
        serde_json::Value::Number(num) => num
            .as_u64()
            .ok_or_else(|| format!("Invalid {label}.")),
        serde_json::Value::String(text) => parse_atom_string(text),
        _ => Err(format!("Invalid {label}.")),
    }
}

/// Sign a reviewed HD contract call from the unlocked session.
/// Uses sighash tag `4` and wire index `6` (`TransactionKind::Call`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn wallet_sign_call_hd(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    bip39_passphrase: Option<String>,
    account_index: Option<u32>,
    contract_address: Option<String>,
    contract: Option<String>,
    counterparty: Option<String>,
    method: String,
    args_hex: Option<String>,
    args: Option<String>,
    value: Option<serde_json::Value>,
    value_atoms: Option<serde_json::Value>,
    amount_atoms: Option<serde_json::Value>,
    gas_limit: serde_json::Value,
    fee: Option<serde_json::Value>,
    fee_atoms: Option<serde_json::Value>,
    nonce: serde_json::Value,
    chain_id: u64,
) -> Result<String, String> {
    let path = wallet_file(&app)?;
    let bip39_passphrase = Zeroizing::new(bip39_passphrase.unwrap_or_default());
    let target = contract_address
        .or(contract)
        .or(counterparty)
        .ok_or_else(|| "Enter a contract address.".to_string())?;
    let calldata_hex = args_hex.or(args).unwrap_or_default();
    let val_json = value
        .or(value_atoms)
        .or(amount_atoms)
        .unwrap_or_else(|| serde_json::Value::from(0_u64));
    let fee_json = fee
        .or(fee_atoms)
        .ok_or_else(|| "Enter a fee amount.".to_string())?;
    let value_u64 = parse_u64_json(&val_json, "value")?;
    let gas_u64 = parse_u64_json(&gas_limit, "gas limit")?;
    let fee_u64 = parse_u64_json(&fee_json, "fee")?;
    let nonce_u64 = parse_u64_json(&nonce, "nonce")?;
    with_wallet(&state, |store| {
        store.sign_call_hd(
            &path,
            bip39_passphrase.as_str(),
            account_index.unwrap_or(0),
            &target,
            &method,
            &calldata_hex,
            value_u64,
            gas_u64,
            fee_u64,
            nonce_u64,
            chain_id,
            Instant::now(),
        )
    })
}

/// Sign a reviewed legacy contract call from the unlocked session.
/// Uses sighash tag `4` and wire index `6` (`TransactionKind::Call`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn wallet_sign_call(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    contract_address: Option<String>,
    contract: Option<String>,
    counterparty: Option<String>,
    method: String,
    args_hex: Option<String>,
    args: Option<String>,
    value: Option<serde_json::Value>,
    value_atoms: Option<serde_json::Value>,
    amount_atoms: Option<serde_json::Value>,
    gas_limit: serde_json::Value,
    fee: Option<serde_json::Value>,
    fee_atoms: Option<serde_json::Value>,
    nonce: serde_json::Value,
    chain_id: u64,
) -> Result<String, String> {
    let path = wallet_file(&app)?;
    let target = contract_address
        .or(contract)
        .or(counterparty)
        .ok_or_else(|| "Enter a contract address.".to_string())?;
    let calldata_hex = args_hex.or(args).unwrap_or_default();
    let val_json = value
        .or(value_atoms)
        .or(amount_atoms)
        .unwrap_or_else(|| serde_json::Value::from(0_u64));
    let fee_json = fee
        .or(fee_atoms)
        .ok_or_else(|| "Enter a fee amount.".to_string())?;
    let value_u64 = parse_u64_json(&val_json, "value")?;
    let gas_u64 = parse_u64_json(&gas_limit, "gas limit")?;
    let fee_u64 = parse_u64_json(&fee_json, "fee")?;
    let nonce_u64 = parse_u64_json(&nonce, "nonce")?;
    with_wallet(&state, |store| {
        store.sign_call(
            &path,
            &target,
            &method,
            &calldata_hex,
            value_u64,
            gas_u64,
            fee_u64,
            nonce_u64,
            chain_id,
            Instant::now(),
        )
    })
}

/// Open an existing wallet. A wrong password does not reveal the phrase.
#[tauri::command]
fn wallet_unlock(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    passphrase: String,
) -> Result<wallet::WalletStatus, String> {
    let path = wallet_file(&app)?;
    let passphrase = Zeroizing::new(passphrase);
    with_wallet(&state, |store| {
        store.unlock(&path, passphrase.as_str(), Instant::now())
    })
}

/// Show the recovery phrase again. The password is required. A wrong password
/// returns no phrase.
#[tauri::command]
fn wallet_reveal(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
    passphrase: String,
) -> Result<wallet::NewWallet, String> {
    let path = wallet_file(&app)?;
    let passphrase = Zeroizing::new(passphrase);
    with_wallet(&state, |store| {
        store.reveal(&path, passphrase.as_str(), Instant::now())
    })
}

/// Drop the session and tell the webview. [`wallet_lock`] calls this.
fn lock_and_emit(
    path: &std::path::Path,
    shared: &wallet::WalletShared,
    notify: &dyn wallet::SessionNotify,
    now: Instant,
) -> Result<wallet::WalletStatus, String> {
    let outcome = wallet::with_store(shared, notify, |store| store.manual_lock(path, now))?;
    wallet::notify_manual_lock(&outcome, notify);
    Ok(outcome.status)
}

/// Drop the session. The webview hears [`wallet::WALLET_LOCKED_EVENT`] at once.
///
/// `R` is generic so the mock runtime can invoke this same command.
#[tauri::command]
fn wallet_lock<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, WalletApp>,
) -> Result<wallet::WalletStatus, String> {
    let path = wallet_file(&app)?;
    lock_and_emit(&path, &state.shared, state.notify.as_ref(), Instant::now())
}

/// Note that the user is still at the keyboard.
///
/// This is the only IPC that moves the idle deadline of an open session.
#[tauri::command]
fn wallet_touch(state: State<'_, WalletApp>) -> Result<(), String> {
    with_wallet(&state, |store| {
        store.touch(Instant::now());
        Ok(())
    })
}

/// Quote a transfer. The keystore is not decrypted.
#[tauri::command]
async fn wallet_quote(
    app: tauri::AppHandle,
    shown_chain_id: Option<String>,
    recipient: String,
    amount: String,
) -> Result<transfer::TransferQuote, String> {
    let url = current_url(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let wallet = app.state::<WalletApp>();
        with_wallet(&wallet, |store| {
            store.quote_on(
                &rpc,
                &url,
                transfer::TransferDraft {
                    shown_chain_id: shown_chain_id.as_deref(),
                    confirm_mainnet: false,
                    recipient: &recipient,
                    amount: &amount,
                },
                Instant::now(),
            )
        })
    })
    .await
    .map_err(|_| "The quote was interrupted.".to_string())?
}

/// Balance the node reported. It is not a light-client proof.
#[tauri::command]
async fn wallet_balance(app: tauri::AppHandle) -> Result<history::ReportedBalance, String> {
    let url = current_url(&app)?;
    let path = wallet_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let wallet = app.state::<WalletApp>();
        let address = with_wallet(&wallet, |store| {
            store
                .status(&path, Instant::now())?
                .address
                .ok_or_else(|| "Create or import a wallet first.".to_string())
        })?;
        history::reported_balance(&rpc, &address)
    })
    .await
    .map_err(|_| "The balance request was interrupted.".to_string())?
}

/// Transfers the node reported for this wallet. They are not proven.
#[tauri::command]
async fn wallet_history(app: tauri::AppHandle) -> Result<history::HistoryPage, String> {
    let url = current_url(&app)?;
    let path = wallet_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let wallet = app.state::<WalletApp>();
        let address = with_wallet(&wallet, |store| {
            store
                .status(&path, Instant::now())?
                .address
                .ok_or_else(|| "Create or import a wallet first.".to_string())
        })?;
        let chain_id = fetch_status(&rpc, &url)
            .ok()
            .and_then(|status| history::proven_chain_id(status.chain_id.as_deref()));
        history::reported_history(&rpc, &address, chain_id)
    })
    .await
    .map_err(|_| "The history request was interrupted.".to_string())?
}

/// Sign a quoted transfer. Chain 1 is refused until `confirm_mainnet` is set.
#[tauri::command]
async fn wallet_send(
    app: tauri::AppHandle,
    shown_chain_id: Option<String>,
    confirm_mainnet: bool,
    recipient: String,
    amount: String,
) -> Result<transfer::SendReceipt, String> {
    let url = current_url(&app)?;
    let path = wallet_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let status = fetch_status(&rpc, &url).map_err(|err| err.to_string())?;
        let wallet = app.state::<WalletApp>();
        with_wallet(&wallet, |store| {
            store.send_prepared(
                &rpc,
                &path,
                &status,
                transfer::TransferDraft {
                    shown_chain_id: shown_chain_id.as_deref(),
                    confirm_mainnet,
                    recipient: &recipient,
                    amount: &amount,
                },
                Instant::now(),
            )
        })
    })
    .await
    .map_err(|_| "The transfer was interrupted.".to_string())?
}

fn parse_atom_string(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Enter an atom amount.".to_string());
    }
    text.parse::<u64>()
        .map_err(|_| "That amount is larger than KUNA can send.".to_string())
}

/// Ask the node for 100 KVNC. Refused on mainnet.
#[tauri::command]
async fn wallet_request_airdrop(
    app: tauri::AppHandle,
    state: State<'_, WalletApp>,
) -> Result<String, String> {
    let url = current_url(&app)?;
    let path = wallet_file(&app)?;
    let address = with_wallet(&state, |store| {
        store
            .status(&path, Instant::now())?
            .address
            .ok_or_else(|| "Unlock the wallet before requesting testnet KVNC.".to_string())
    })?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let status = fetch_status(&rpc, &url).map_err(|err| err.to_string())?;
        let sync = rpc
            .call("kovanica_syncStatus", serde_json::json!([]))
            .map_err(|err| err.to_string())?;
        if faucet_chain_id(status.chain_id.as_deref(), &sync).is_none() {
            return Err("The faucet is not available on this chain.".to_string());
        }
        let result = rpc
            .call(
                "kovanica_requestAirdrop",
                serde_json::json!([address, "100000000000"]),
            )
            .map_err(|err| err.to_string())?;
        airdrop_hash(&result).ok_or_else(|| "The faucet did not return a transaction hash.".to_string())
    })
    .await
    .map_err(|_| "The faucet request was interrupted.".to_string())?
}

/// Chain the testnet faucet may credit, or nothing on mainnet and chain 4.
#[tauri::command]
async fn wallet_faucet_chain(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let url = current_url(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let status = fetch_status(&rpc, &url).map_err(|err| err.to_string())?;
        let sync = rpc
            .call("kovanica_syncStatus", serde_json::json!([]))
            .map_err(|err| err.to_string())?;
        Ok(faucet_chain_id(status.chain_id.as_deref(), &sync))
    })
    .await
    .map_err(|_| "The faucet request was interrupted.".to_string())?
}

/// Encrypted keystore only. The recovery phrase is not in this file.
#[tauri::command]
fn wallet_export_backup(app: tauri::AppHandle) -> Result<String, String> {
    let path = wallet_file(&app)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "Create a wallet before exporting a backup.".to_string())?;
    let keystore: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| "The keystore could not be read.".to_string())?;
    if keystore.get("mnemonic").is_some() || keystore.get("phrase").is_some() {
        return Err("Refusing to export a file that contains a recovery phrase.".to_string());
    }
    Ok(serde_json::json!({ "version": 1, "keystore": keystore }).to_string())
}


/// Nonce and fee for a reviewed contract call. This does not sign.
#[tauri::command]
async fn wallet_call_quote(app: tauri::AppHandle) -> Result<transfer::CallQuote, String> {
    let url = current_url(&app)?;
    let path = wallet_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone()).map_err(|err| err.to_string())?;
        let wallet = app.state::<WalletApp>();
        let address = with_wallet(&wallet, |store| {
            store
                .status(&path, Instant::now())?
                .address
                .ok_or_else(|| "Create or import a wallet first.".to_string())
        })?;
        transfer::quote_call(&rpc, &url, &address)
    })
    .await
    .map_err(|_| "The quote was interrupted.".to_string())?
}

/// Submit a raw transaction the user already reviewed and signed.
#[tauri::command]
async fn wallet_broadcast_raw(app: tauri::AppHandle, raw: String) -> Result<String, String> {
    let url = current_url(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url).map_err(|err| err.to_string())?;
        transfer::broadcast_raw(&rpc, &raw)
    })
    .await
    .map_err(|_| "The broadcast was interrupted.".to_string())?
}

/// Read one contract storage slot. The slot is a 32-byte hex key.
#[tauri::command]
async fn wallet_storage_at(
    app: tauri::AppHandle,
    address: String,
    slot: String,
) -> Result<Option<String>, String> {
    if address.trim().is_empty() || address.len() > 120 {
        return Err("Enter a contract address.".to_string());
    }
    let slot_trim = slot.trim();
    let body = slot_trim
        .strip_prefix("0x")
        .or_else(|| slot_trim.strip_prefix("0X"))
        .unwrap_or(slot_trim);
    if body.len() != 64 || !body.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Storage slot must be 32 bytes of hex.".to_string());
    }
    let slot_hex = format!("0x{}", body.to_ascii_lowercase());
    let url = current_url(&app)?;
    let address = address.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url).map_err(|err| err.to_string())?;
        transfer::storage_at(&rpc, &address, &slot_hex)
    })
    .await
    .map_err(|_| "The storage read was interrupted.".to_string())?
}

/// Chain id, height, leader round, epoch, and peers. Fails with a clear error
/// when the node cannot be reached.
#[tauri::command]
async fn node_status(app: tauri::AppHandle) -> Result<NodeStatus, String> {
    let url = current_url(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let rpc = HttpRpc::new(url.clone())?;
        fetch_status(&rpc, &url)
    })
    .await
    .map_err(|_| "The status request was interrupted.".to_string())?
    .map_err(|err| err.to_string())
}

/// Forwards a session lock to the webview. The idle thread calls this.
struct EmitSession<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> wallet::SessionNotify for EmitSession<R> {
    fn wallet_locked(&self) {
        let _ = self.app.emit(wallet::WALLET_LOCKED_EVENT, ());
    }

    fn wallet_lock_warning(&self) {
        let _ = self.app.emit(wallet::WALLET_LOCK_WARNING_EVENT, ());
    }
}

/// Session, idle thread, and the notifier that emits `wallet-locked`.
fn install_wallet<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> WalletApp {
    let shared = Arc::new(wallet::WalletShared::new());
    let notify: Arc<dyn wallet::SessionNotify> = Arc::new(EmitSession { app: app.clone() });
    let idle = wallet::IdleLockThread::spawn_with(Arc::clone(&shared), Arc::clone(&notify));
    WalletApp {
        idle,
        shared,
        notify,
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(install_wallet(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connection_config,
            save_connection,
            reset_connection,
            node_status,
            parse_kuna,
            format_kuna,
            wallet_status,
            wallet_create,
            wallet_import,
            wallet_create_hd,
            wallet_import_hd,
            wallet_derive_account,
            wallet_sign_transfer_hd,
            wallet_sign_call_hd,
            wallet_sign_call,
            wallet_unlock,
            wallet_reveal,
            wallet_lock,
            wallet_touch,
            wallet_quote,
            wallet_send,
            wallet_balance,
            wallet_history,
            wallet_request_airdrop,
            wallet_faucet_chain,
            wallet_export_backup,
            wallet_call_quote,
            wallet_broadcast_raw,
            wallet_storage_at
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kovanica Console");
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{install_wallet, lock_and_emit};
    use tauri::Listener;
    use tauri::Manager;

    /// The lock command's emitter is the app bus, not a string in `main.rs`.
    #[test]
    fn wallet_lock_emits_wallet_locked_on_the_app() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("test app");
        let wallet = install_wallet(app.handle());
        let now = std::time::Instant::now();
        wallet
            .shared
            .lock_store()
            .open_for_test("kvnc-emit".into(), now);
        assert!(wallet.shared.lock_store().is_open_for_test());

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&hits);
        app.listen(crate::wallet::WALLET_LOCKED_EVENT, move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "kovanica-console-missing-wallet-{}-{}",
            std::process::id(),
            nanos
        ));
        assert!(!path.exists());

        let status =
            lock_and_emit(&path, &wallet.shared, wallet.notify.as_ref(), now).expect("lock");
        assert_eq!(status.phase, "absent");
        assert!(!wallet.shared.lock_store().is_open_for_test());
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let again =
            lock_and_emit(&path, &wallet.shared, wallet.notify.as_ref(), now).expect("second lock");
        assert_eq!(again.phase, "absent");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        drop(wallet);
    }

    /// The `wallet_lock` command itself emits `wallet-locked`, and a second
    /// call with no session does not emit it again.
    #[test]
    fn wallet_lock_command_emits_wallet_locked_once() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "kovanica-console-lock-ipc-{}-{}",
            std::process::id(),
            nanos
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _cleanup = RemoveDir(root.clone());

        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().app.app_directories_override = Some(
            tauri::utils::config::AppDirectoriesOverride::Root(root.clone()),
        );

        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![super::wallet_lock])
            .build(context)
            .expect("test app");
        app.manage(install_wallet(app.handle()));

        let now = std::time::Instant::now();
        {
            let wallet = app.state::<super::WalletApp>();
            wallet
                .shared
                .lock_store()
                .open_for_test("kvnc-lock-cmd".into(), now);
            assert!(wallet.shared.lock_store().is_open_for_test());
        }

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&hits);
        app.listen(crate::wallet::WALLET_LOCKED_EVENT, move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });

        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview");

        let first = tauri::test::get_ipc_response(&webview, wallet_lock_request())
            .unwrap_or_else(|err| panic!("wallet_lock failed: {err}"));
        let first = response_json(first);
        assert_eq!(first["phase"], "absent");
        assert!(first["address"].is_null());
        assert!(root.is_dir());
        assert!(!app
            .state::<super::WalletApp>()
            .shared
            .lock_store()
            .is_open_for_test());
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let second = tauri::test::get_ipc_response(&webview, wallet_lock_request())
            .unwrap_or_else(|err| panic!("second wallet_lock failed: {err}"));
        let second = response_json(second);
        assert_eq!(second["phase"], "absent");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    fn wallet_lock_request() -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: "wallet_lock".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().expect("local url"),
            body: tauri::ipc::InvokeBody::Json(serde_json::json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    fn response_json(body: tauri::ipc::InvokeResponseBody) -> serde_json::Value {
        match body {
            tauri::ipc::InvokeResponseBody::Json(text) => {
                serde_json::from_str(&text).expect("wallet_lock json")
            }
            tauri::ipc::InvokeResponseBody::Raw(_) => panic!("wallet_lock returned bytes"),
        }
    }

    /// Deletes a test directory when the test ends, including on failure.
    struct RemoveDir(std::path::PathBuf);

    impl Drop for RemoveDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
