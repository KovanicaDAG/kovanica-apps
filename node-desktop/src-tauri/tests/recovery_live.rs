//! Live chain-5 recovery for the Linux supervisor and its monitor.
//!
//! Not part of `cargo test --lib`. `scripts/recovery-suite.sh` runs it only
//! after the mocked regressions. Missing prerequisites exit 2 (BLOCKED) and
//! are not a pass. Chain 1337 is not a substitute. `KOVANICA_TEST_TARGET_BITS`
//! is never set.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use kovanica_node_desktop::monitor::{self, Field, RpcReach};
use kovanica_node_desktop::policy::check_setup;
use kovanica_node_desktop::session::{BeginPoll, Session};
use kovanica_node_desktop::supervisor::{ProcessState, STOP_GRACE};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const AUTHOR0: &str = "kvncqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqz4k5rcdag";
const EASY_TEST_TARGET: &str = "0x207fffff";
const FIRST_BLOCK_WAIT: Duration = Duration::from_secs(180);
const ADVANCE_WAIT: Duration = Duration::from_secs(180);

fn main_outcome() {
    let mut cleanup = Cleanup::default();
    match run(&mut cleanup) {
        Ok(report) => {
            cleanup.shutdown();
            write_report(&report);
            eprintln!("live recovery passed");
        }
        Err(Stop::Blocked(reason)) => {
            cleanup.shutdown();
            write_report(&json!({
                "suite": "node-desktop-recovery",
                "result": "blocked",
                "blocked": reason,
                "limitations": limitations(),
                "checks": [],
            }));
            eprintln!("BLOCKED: {reason}");
            std::process::exit(2);
        }
        Err(Stop::Failed(reason)) => {
            cleanup.shutdown();
            write_report(&json!({
                "suite": "node-desktop-recovery",
                "result": "fail",
                "failure": reason,
                "limitations": limitations(),
                "checks": [],
            }));
            panic!("live recovery failed: {reason}");
        }
    }
}

#[test]
fn live_chain5_recovery() {
    main_outcome();
}

enum Stop {
    Blocked(String),
    Failed(String),
}

#[derive(Default)]
struct Cleanup {
    session: Option<Session>,
    producer: Option<Child>,
    bystander: Option<Child>,
    dir: Option<tempfile::TempDir>,
    pids: Vec<u32>,
}

impl Cleanup {
    fn note(&mut self, pid: u32) {
        self.pids.push(pid);
        let _ = write_pids(&self.pids);
    }

    fn shutdown(&mut self) {
        if let Some(session) = self.session.as_ref() {
            let _ = session.stop_owned();
        }
        if let Some(child) = self.producer.as_mut() {
            let pid = child.id();
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
                libc::kill(pid as i32, libc::SIGKILL);
            }
            let _ = child.wait();
        }
        self.producer = None;
        if let Some(child) = self.bystander.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.bystander = None;
        self.dir.take();
        let _ = fs::remove_file(artifact_dir().join("live.pids"));
    }
}

fn run(cleanup: &mut Cleanup) -> Result<Value, Stop> {
    if kov_names()
        .iter()
        .any(|name| name == "KOVANICA_TEST_TARGET_BITS")
    {
        return Err(Stop::Blocked(
            "KOVANICA_TEST_TARGET_BITS is set. This suite will not use a runtime difficulty override. Unset it. The value is not shown.".into(),
        ));
    }
    let foreign: Vec<_> = kov_names()
        .into_iter()
        .filter(|name| name != "KOVANICA_NODE_BIN" && name != "KOVANICA_RECOVERY_FUNDED_ACCOUNTS")
        .collect();
    if !foreign.is_empty() {
        return Err(Stop::Blocked(format!(
            "KOVANICA_* is set ({}). Values are not shown. Unset them. Chain 1337 is not a substitute and no difficulty override is enabled.",
            foreign.join(", ")
        )));
    }
    let binary = required_absolute("NODE_RECOVERY_BIN", true)?;
    let funded = required_absolute("NODE_RECOVERY_FUNDED_ACCOUNTS", false)?;
    let sha = sha256_file(&binary).map_err(Stop::Failed)?;
    let version = version_probe(&binary);
    let funded_text = fs::read_to_string(&funded)
        .map_err(|err| Stop::Failed(format!("could not read funded accounts: {err}")))?;
    let funded_sha = sha256_bytes(funded_text.as_bytes());
    let (faucet, faucet_balance) = funded_account(&funded_text)?;

    let dir = tempfile::tempdir().map_err(|err| Stop::Failed(err.to_string()))?;
    let root = dir.path().to_path_buf();
    cleanup.dir = Some(dir);
    let producer_data = root.join("producer-data");
    let observer_data = root.join("observer-data");
    fs::create_dir_all(&producer_data).map_err(|err| Stop::Failed(err.to_string()))?;
    fs::create_dir_all(&observer_data).map_err(|err| Stop::Failed(err.to_string()))?;
    fs::write(producer_data.join("funded_accounts.toml"), &funded_text)
        .map_err(|err| Stop::Failed(err.to_string()))?;
    fs::write(observer_data.join("funded_accounts.toml"), &funded_text)
        .map_err(|err| Stop::Failed(err.to_string()))?;
    let key_path = producer_data.join("validator.key");
    write_key(&key_path)?;

    let producer_p2p = free_port()?;
    let producer_rpc = free_port()?;
    let observer_p2p = free_port()?;
    let observer_rpc = free_port()?;
    let producer_toml = root.join("producer.toml");
    let observer_toml = root.join("observer.toml");
    let producer_body = format!(
        "chain_id = 5\n\
         data_dir = \"{}\"\n\
         listen_addr = \"127.0.0.1:{producer_p2p}\"\n\
         rpc_addr = \"127.0.0.1\"\n\
         rpc_port = {producer_rpc}\n\
         bootnodes = []\n\
         run_validator = true\n\
         validator_key = \"{}\"\n\
         max_peers = 20\n\
         rpc_rate_limit_per_min = 0\n\
         \n\
         [mining]\n\
         enabled = true\n\
         threads = 1\n\
         author = 0\n",
        producer_data.display(),
        key_path.display()
    );
    let observer_body = format!(
        "chain_id = 5\n\
         data_dir = \"{}\"\n\
         listen_addr = \"127.0.0.1:{observer_p2p}\"\n\
         rpc_addr = \"127.0.0.1\"\n\
         rpc_port = {observer_rpc}\n\
         bootnodes = [\"127.0.0.1:{producer_p2p}\"]\n\
         run_validator = false\n\
         max_peers = 20\n\
         rpc_rate_limit_per_min = 0\n\
         \n\
         [mining]\n\
         enabled = false\n",
        observer_data.display()
    );
    fs::write(&producer_toml, &producer_body).map_err(|err| Stop::Failed(err.to_string()))?;
    fs::write(&observer_toml, &observer_body).map_err(|err| Stop::Failed(err.to_string()))?;
    let observer_sha_before = sha256_bytes(observer_body.as_bytes());

    let bystander = Command::new("sleep")
        .arg("600")
        .spawn()
        .map_err(|err| Stop::Failed(format!("could not start bystander: {err}")))?;
    let bystander_pid = bystander.id();
    cleanup.note(bystander_pid);
    cleanup.bystander = Some(bystander);

    let producer_log =
        File::create(root.join("producer.log")).map_err(|err| Stop::Failed(err.to_string()))?;
    let producer_err = producer_log
        .try_clone()
        .map_err(|err| Stop::Failed(err.to_string()))?;
    let mut producer = Command::new(&binary);
    producer
        .arg("--config")
        .arg(&producer_toml)
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin".into()),
        )
        .env(
            "HOME",
            std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()),
        )
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::from(producer_log))
        .stderr(Stdio::from(producer_err));
    unsafe {
        producer.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let producer = producer
        .spawn()
        .map_err(|err| Stop::Failed(format!("could not start producer: {err}")))?;
    let producer_pid = producer.id();
    cleanup.note(producer_pid);
    cleanup.producer = Some(producer);
    assert_no_kovanica_env(producer_pid, "producer")?;

    let producer_base = format!("http://127.0.0.1:{producer_rpc}");
    if !wait_health(&producer_base, Duration::from_secs(30)) {
        return Err(Stop::Failed(format!(
            "producer did not open RPC on {producer_base}. log: {}",
            tail(&root.join("producer.log"))
        )));
    }
    let producer_mining = rpc_value(&producer_base, "kovanica_getMiningInfo", json!([]))?;
    let target_bits = producer_mining
        .get("target_bits")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if target_bits.eq_ignore_ascii_case(EASY_TEST_TARGET) {
        return Err(Stop::Failed(format!(
            "producer target_bits is {EASY_TEST_TARGET}, the isolated-test override. This suite did not set KOVANICA_TEST_TARGET_BITS."
        )));
    }

    let checked = check_setup(
        &binary,
        &observer_toml,
        &observer_data,
        &kov_names_for_check(),
    )
    .map_err(|err| {
        if err.contains("would override") {
            Stop::Blocked(err)
        } else {
            Stop::Failed(err)
        }
    })?;
    if checked.chain_id != 5 || checked.mining_enabled || checked.run_validator {
        return Err(Stop::Failed(
            "check_setup did not keep chain 5, mining off, and run_validator false".into(),
        ));
    }
    let session = Session::new(STOP_GRACE);
    session
        .supervisor
        .start(Path::new(&checked.binary), Path::new(&checked.config_path))
        .map_err(|err| Stop::Failed(format!("supervisor start failed: {err}")))?;
    let observer_pid = session
        .supervisor
        .status()
        .pid
        .ok_or_else(|| Stop::Failed("supervisor did not publish a pid".into()))?;
    cleanup.note(observer_pid);
    if observer_pid == producer_pid || observer_pid == bystander_pid {
        return Err(Stop::Failed(
            "supervised pid collided with the producer or the bystander".into(),
        ));
    }
    session.attach_checked(checked);
    cleanup.session = Some(session);
    assert_no_kovanica_env(observer_pid, "observer")?;

    let observer_base = format!("http://127.0.0.1:{observer_rpc}");
    if !wait_health(&observer_base, Duration::from_secs(30)) {
        return Err(Stop::Failed(format!(
            "supervised observer did not open {observer_base}"
        )));
    }
    let observer_mining = rpc_value(&observer_base, "kovanica_getMiningInfo", json!([]))?;
    if observer_mining.get("mining_enabled") != Some(&Value::Bool(false)) {
        return Err(Stop::Failed(format!(
            "supervised node reports mining enabled: {observer_mining}"
        )));
    }

    let before = wait_progress(&observer_base, &faucet, 0, 0, FIRST_BLOCK_WAIT)?;
    if before.committed == 0 || before.balance == 0 || before.dag_round < before.committed {
        return Err(Stop::Failed(format!(
            "nonzero admitted/committed/executed state was not reached: {before:?}"
        )));
    }
    if !before.tag.starts_with("5-") {
        return Err(Stop::Failed(format!(
            "network tag {} is not chain 5. Chain 1337 was not requested.",
            before.tag
        )));
    }
    if before.faucet != faucet_balance {
        return Err(Stop::Failed(format!(
            "funded account balance {} != file balance {faucet_balance}",
            before.faucet
        )));
    }
    publish_monitor(cleanup.session.as_ref().unwrap(), &before)?;

    unsafe {
        libc::kill(-(producer_pid as i32), libc::SIGSTOP);
    }
    thread::sleep(Duration::from_secs(2));
    let paused_a = sample(&observer_base, &faucet)?;
    thread::sleep(Duration::from_secs(1));
    let checkpoint = sample(&observer_base, &faucet)?;
    if checkpoint.committed != paused_a.committed || checkpoint.balance != paused_a.balance {
        return Err(Stop::Failed(
            "observer still advanced while the producer was paused; checkpoint is not stable"
                .into(),
        ));
    }
    if checkpoint.committed < before.committed || checkpoint.balance < before.balance {
        return Err(Stop::Failed(
            "paused observer lost committed or executed state before stop".into(),
        ));
    }

    cleanup
        .session
        .as_ref()
        .unwrap()
        .stop_owned()
        .map_err(|err| Stop::Failed(format!("stop failed: {err}")))?;
    let stopped = cleanup.session.as_ref().unwrap().snapshot();
    if stopped.process.state != ProcessState::Exited {
        return Err(Stop::Failed(format!(
            "stop left the supervisor in {:?}",
            stopped.process.state
        )));
    }
    if !alive(bystander_pid) {
        return Err(Stop::Failed(
            "stopping the supervised node killed the bystander".into(),
        ));
    }
    if alive(observer_pid) {
        return Err(Stop::Failed(
            "supervised node was still alive after stop".into(),
        ));
    }
    let observer_sha_after = sha256_file(&observer_toml).map_err(Stop::Failed)?;
    if observer_sha_after != observer_sha_before {
        return Err(Stop::Failed(
            "observer config was rewritten. The file was not supposed to change.".into(),
        ));
    }

    cleanup
        .session
        .as_ref()
        .unwrap()
        .supervisor
        .start(Path::new(&binary), &observer_toml)
        .map_err(|err| Stop::Failed(format!("restart failed: {err}")))?;
    let restarted_pid = cleanup
        .session
        .as_ref()
        .unwrap()
        .supervisor
        .status()
        .pid
        .ok_or_else(|| Stop::Failed("restart did not publish a pid".into()))?;
    cleanup.note(restarted_pid);
    if restarted_pid == bystander_pid || restarted_pid == producer_pid {
        return Err(Stop::Failed("restart pid collided".into()));
    }
    let names = kov_names_for_check();
    let checked =
        check_setup(&binary, &observer_toml, &observer_data, &names).map_err(Stop::Failed)?;
    cleanup.session.as_ref().unwrap().attach_checked(checked);
    let session = cleanup.session.as_ref().unwrap();
    if !wait_health(&observer_base, Duration::from_secs(30)) {
        return Err(Stop::Failed(
            "restarted observer RPC did not come back while the producer stayed paused".into(),
        ));
    }
    let recovered = sample(&observer_base, &faucet)?;
    if recovered.peers != 0 {
        return Err(Stop::Failed(format!(
            "restart saw {} peers while the producer was paused; that is not disk recovery",
            recovered.peers
        )));
    }
    if recovered.committed != checkpoint.committed || recovered.balance != checkpoint.balance {
        return Err(Stop::Failed(format!(
            "disk recovery mismatch committed {} vs {} balance {} vs {}",
            recovered.committed, checkpoint.committed, recovered.balance, checkpoint.balance
        )));
    }
    if recovered.dag_round < checkpoint.committed {
        return Err(Stop::Failed(format!(
            "admitted DAG round {} is behind persisted committed height {}",
            recovered.dag_round, checkpoint.committed
        )));
    }
    if recovered.faucet != faucet_balance {
        return Err(Stop::Failed(
            "funded balance changed across restart with the producer paused".into(),
        ));
    }
    publish_monitor(session, &recovered)?;
    let shown = session.snapshot();
    if shown.monitor.sync.committed_height != Field::value(recovered.committed.to_string()) {
        return Err(Stop::Failed(format!(
            "monitor committed height {:?} != RPC {}",
            shown.monitor.sync.committed_height, recovered.committed
        )));
    }
    if shown.monitor.rpc != RpcReach::Reachable || shown.monitor.stale {
        return Err(Stop::Failed(
            "recovered monitor was not a current reachable read".into(),
        ));
    }

    unsafe {
        libc::kill(-(producer_pid as i32), libc::SIGCONT);
    }
    let advanced = wait_progress(
        &observer_base,
        &faucet,
        checkpoint.committed,
        checkpoint.balance,
        ADVANCE_WAIT,
    )?;
    if advanced.dag_round < advanced.committed || advanced.dag_round <= checkpoint.dag_round {
        return Err(Stop::Failed(format!(
            "admitted tip did not move past the recovered checkpoint: {advanced:?}"
        )));
    }
    publish_monitor(session, &advanced)?;
    session
        .stop_owned()
        .map_err(|err| Stop::Failed(format!("final stop failed: {err}")))?;
    if !alive(bystander_pid) {
        return Err(Stop::Failed("final stop killed the bystander".into()));
    }

    Ok(json!({
        "suite": "node-desktop-recovery",
        "result": "pass",
        "binary": {
            "path": binary.display().to_string(),
            "sha256": sha,
            "version_probe": version,
            "features": "not exposed by the CLI. This suite does not infer debug, release, or a DAA fix from the path or from any L1 checkout.",
        },
        "network": {
            "chain_id": 5,
            "network_tag": checkpoint.tag,
            "funded_accounts_sha256": funded_sha,
            "funded_address": faucet,
            "producer_target_bits": target_bits,
            "test_target_bits": "unset",
        },
        "sanitized_config": {
            "observer": observer_body,
            "producer": sanitize_producer(&producer_body),
        },
        "checks": [
            {"name": "admitted_committed_executed", "outcome": "pass", "committed": before.committed, "dag_round": before.dag_round, "author0_balance": before.balance},
            {"name": "paused_checkpoint", "outcome": "pass", "committed": checkpoint.committed, "dag_round": checkpoint.dag_round, "author0_balance": checkpoint.balance, "peers": checkpoint.peers},
            {"name": "disk_recovery_producer_paused", "outcome": "pass", "committed": recovered.committed, "dag_round": recovered.dag_round, "author0_balance": recovered.balance, "peers": recovered.peers, "faucet_balance": recovered.faucet},
            {"name": "advance_after_resume", "outcome": "pass", "committed": advanced.committed, "dag_round": advanced.dag_round, "author0_balance": advanced.balance},
            {"name": "bystander_survived", "outcome": "pass", "pid": bystander_pid},
            {"name": "config_not_rewritten", "outcome": "pass", "sha256": observer_sha_after},
        ],
        "limitations": limitations(),
        "producer_log_tail": tail(&root.join("producer.log")),
    }))
}

#[derive(Debug)]
struct Sample {
    committed: u64,
    dag_round: u64,
    balance: u64,
    faucet: u64,
    peers: u64,
    tag: String,
}

fn wait_progress(
    base: &str,
    faucet: &str,
    min_committed: u64,
    min_balance: u64,
    bound: Duration,
) -> Result<Sample, Stop> {
    let start = Instant::now();
    let mut last = String::new();
    loop {
        if start.elapsed() > bound {
            return Err(Stop::Failed(format!(
                "timed out after {}s waiting for committed > {min_committed} and author-0 balance > {min_balance}. last: {last}",
                bound.as_secs()
            )));
        }
        match sample(base, faucet) {
            Ok(sample) => {
                last = format!("{sample:?}");
                if sample.committed > min_committed
                    && sample.balance > min_balance
                    && sample.dag_round >= sample.committed
                    && !sample.tag.is_empty()
                {
                    return Ok(sample);
                }
            }
            Err(Stop::Blocked(reason)) => return Err(Stop::Blocked(reason)),
            Err(Stop::Failed(reason)) => last = reason,
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn sample(base: &str, faucet: &str) -> Result<Sample, Stop> {
    let sync = rpc_value(base, "kovanica_syncStatus", json!([]))?;
    let tips = rpc_value(base, "kovanica_getDagTips", json!([]))?;
    let balance = rpc_quantity(base, "kovanica_getBalance", json!([AUTHOR0]))?;
    let faucet_balance = rpc_quantity(base, "kovanica_getBalance", json!([faucet]))?;
    let committed = json_u64(sync.get("committedHeight")).ok_or_else(|| {
        Stop::Failed("kovanica_syncStatus has no committedHeight. Height was not invented.".into())
    })?;
    let dag_round = json_u64(tips.get("max_round")).ok_or_else(|| {
        Stop::Failed(
            "kovanica_getDagTips has no max_round. Admitted state was not invented.".into(),
        )
    })?;
    let peers = json_u64(sync.get("peerCount")).unwrap_or(0);
    let tag = sync
        .get("networkTag")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(Sample {
        committed,
        dag_round,
        balance,
        faucet: faucet_balance,
        peers,
        tag,
    })
}

fn publish_monitor(session: &Session, sample: &Sample) -> Result<(), Stop> {
    let now = Instant::now();
    let BeginPoll::Fetch {
        generation,
        rpc_http,
    } = session.begin_monitor_poll(now)
    else {
        return Err(Stop::Failed(
            "monitor poll did not start while the supervised node was running".into(),
        ));
    };
    let view = tokio_run(monitor::fetch_monitor(&rpc_http))
        .map_err(|err| Stop::Failed(format!("integrated monitor read failed: {err}")))?;
    session.finish_monitor_ok(generation, Instant::now(), view);
    let shown = session.snapshot();
    if shown.monitor.sync.committed_height != Field::value(sample.committed.to_string()) {
        return Err(Stop::Failed(format!(
            "monitor shows {:?} after a fresh read, RPC committed {}",
            shown.monitor.sync.committed_height, sample.committed
        )));
    }
    if shown.monitor.last_success_age_ms != Some(0) || shown.monitor.stale {
        return Err(Stop::Failed("fresh monitor read was not current".into()));
    }
    Ok(())
}

fn rpc_value(base: &str, method: &str, params: Value) -> Result<Value, Stop> {
    let url = format!("{base}/rpc");
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let text = tokio_run(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|err| err.to_string())?;
        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        response.text().await.map_err(|err| err.to_string())
    })
    .map_err(|err| Stop::Failed(format!("{method} transport: {err}")))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| Stop::Failed(format!("{method} returned non-JSON")))?;
    if let Some(error) = value.get("error") {
        if error.get("code").and_then(Value::as_i64) == Some(-32601) {
            return Err(Stop::Blocked(format!(
                "selected node does not support {method}. This suite does not invent a replacement field."
            )));
        }
        return Err(Stop::Failed(format!("{method} error: {error}")));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| Stop::Failed(format!("{method} returned no result")))
}

fn rpc_quantity(base: &str, method: &str, params: Value) -> Result<u64, Stop> {
    let value = rpc_value(base, method, params)?;
    json_u64(Some(&value))
        .ok_or_else(|| Stop::Failed(format!("{method} result was not a quantity: {value}")))
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    let text = value.as_str()?;
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u64::from_str_radix(text, 16).ok()
}

fn wait_health(base: &str, bound: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < bound {
        let ok = tokio_run(async {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(|err| err.to_string())?;
            let response = client
                .get(format!("{base}/health"))
                .send()
                .await
                .map_err(|err| err.to_string())?;
            Ok::<bool, String>(response.status().is_success())
        });
        if matches!(ok, Ok(true)) {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

fn tokio_run<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| err.to_string())?
        .block_on(future)
}

fn funded_account(text: &str) -> Result<(String, u64), Stop> {
    let doc: toml::Value = toml::from_str(text)
        .map_err(|err| Stop::Blocked(format!("funded accounts file is not TOML: {err}")))?;
    let Some(accounts) = doc.get("account").and_then(toml::Value::as_array) else {
        return Err(Stop::Blocked(
            "funded accounts file has no [[account]] entry. This suite does not invent balances."
                .into(),
        ));
    };
    let Some(first) = accounts.first() else {
        return Err(Stop::Blocked(
            "funded accounts file has an empty account list".into(),
        ));
    };
    let address = first
        .get("address")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| Stop::Blocked("funded account is missing address".into()))?;
    let balance = first
        .get("balance")
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| Stop::Blocked("funded account balance is not an integer".into()))?;
    if balance < 0 {
        return Err(Stop::Blocked("funded account balance is negative".into()));
    }
    Ok((address.to_string(), balance as u64))
}

fn required_absolute(name: &str, executable: bool) -> Result<PathBuf, Stop> {
    let value = std::env::var(name).unwrap_or_default();
    if value.is_empty() {
        let hint = if name == "NODE_RECOVERY_BIN" {
            "Pass the absolute path of a kovanica-node binary. This suite does not select the active L1 checkout and does not treat chain 1337 as recovery."
        } else {
            "Point it at the chain-5 funded_accounts.toml published for the node image (L1 ops/docker/chain5/funded_accounts.toml). This suite does not embed that file."
        };
        return Err(Stop::Blocked(format!("{name} is unset. {hint}")));
    }
    let path = PathBuf::from(&value);
    if !path.is_absolute() {
        return Err(Stop::Blocked(format!(
            "{name} must be an absolute path. A relative path is not assumed to be a released binary."
        )));
    }
    if executable {
        if !path.is_file() {
            return Err(Stop::Blocked(format!(
                "{name} is not a file: {}",
                path.display()
            )));
        }
        let mode = fs::metadata(&path)
            .map(|meta| {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false);
        if !mode {
            return Err(Stop::Blocked(format!(
                "{name} is not executable: {}",
                path.display()
            )));
        }
    } else if !path.is_file() {
        return Err(Stop::Blocked(format!(
            "{name} is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn kov_names() -> Vec<String> {
    let mut names: Vec<String> = std::env::vars()
        .map(|(key, _)| key)
        .filter(|key| key.starts_with("KOVANICA_"))
        .collect();
    names.sort();
    names.dedup();
    names
}

fn kov_names_for_check() -> Vec<String> {
    kov_names()
        .into_iter()
        .filter(|name| name != "KOVANICA_NODE_BIN" && name != "KOVANICA_RECOVERY_FUNDED_ACCOUNTS")
        .collect()
}

fn assert_no_kovanica_env(pid: u32, who: &str) -> Result<(), Stop> {
    let text = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    let names: Vec<String> = text
        .split(|byte| *byte == 0)
        .filter_map(|item| std::str::from_utf8(item).ok())
        .filter_map(|item| item.split_once('=').map(|(key, _)| key))
        .filter(|key| key.starts_with("KOVANICA_"))
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        Ok(())
    } else {
        Err(Stop::Failed(format!(
            "{who} inherited KOVANICA_* ({}); values are not shown",
            names.join(", ")
        )))
    }
}

fn write_key(path: &Path) -> Result<(), Stop> {
    let mut raw = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut raw))
        .map_err(|err| Stop::Failed(format!("could not generate validator seed: {err}")))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| Stop::Failed(format!("could not create validator seed: {err}")))?;
    writeln!(file, "{}", hex_encode(&raw))
        .map_err(|err| Stop::Failed(format!("could not write validator seed: {err}")))?;
    Ok(())
}

fn free_port() -> Result<u16, Stop> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|err| Stop::Failed(format!("could not bind a loopback port: {err}")))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| Stop::Failed(err.to_string()))
}

fn version_probe(binary: &Path) -> Value {
    let mut command = Command::new(binary);
    command
        .arg("--version")
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin".into()),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => return json!({"spawn_error": err.to_string()}),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut pipe) = child.stdout.take() {
                    let _ = pipe.read_to_string(&mut out);
                }
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut out);
                }
                return json!({
                    "argv": ["--version"],
                    "exit": status.code(),
                    "output": redact(&truncate(&out, 400)),
                });
            }
            Ok(None) if start.elapsed() > Duration::from_secs(3) => {
                let _ = child.kill();
                let _ = child.wait();
                return json!({
                    "argv": ["--version"],
                    "timed_out": true,
                    "note": "killed after 3s. The CLI does not identify a release or a DAA fix.",
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(30)),
            Err(err) => return json!({"wait_error": err.to_string()}),
        }
    }
}

fn sanitize_producer(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.trim_start().starts_with("validator_key") {
                "validator_key = \"<disposable-0600-seed>\""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn limitations() -> Vec<&'static str> {
    vec![
        "Binary SHA-256 identifies bytes only. Cargo features and whether those bytes contain a merged DAA fix are not exposed by the CLI and are not inferred from a checkout.",
        "Chain 1337 is not used. KOVANICA_TEST_TARGET_BITS is not set.",
        "The supervised process stays non-mining and non-validator. The producer is a second process of the same binary.",
        "kovanica_getDagTips max_round is admitted DAG state. kovanica_syncStatus committedHeight is committed state. kovanica_getBalance is executed state. Height alone is not a pass.",
        "/health execution counters are process-local and are not the persisted executed balance.",
        "The monitor's committed height is not labeled as executed-state recovery.",
    ]
}

fn artifact_dir() -> PathBuf {
    std::env::var("RECOVERY_ARTIFACT_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("recovery-artifacts"))
}

fn write_report(value: &Value) {
    let dir = artifact_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("report.json"), format!("{value:#}\n"));
}

fn write_pids(pids: &[u32]) -> std::io::Result<()> {
    let dir = artifact_dir();
    fs::create_dir_all(&dir)?;
    let mut file = File::create(dir.join("live.pids"))?;
    for pid in pids {
        writeln!(file, "{pid}")?;
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|err| format!("could not hash {}: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|err| err.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(hasher.finalize().as_slice()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(digest.as_slice())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn tail(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(40);
    redact(&lines[start..].join("\n"))
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}…", &text[..max])
    }
}

fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_hexdigit() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_hexdigit() {
                index += 1;
            }
            if index - start >= 32 {
                out.push_str("[redacted]");
            } else {
                out.push_str(&text[start..index]);
            }
        } else {
            let start = index;
            while index < bytes.len() && !bytes[index].is_ascii_hexdigit() {
                index += 1;
            }
            out.push_str(&text[start..index]);
        }
    }
    out
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.shutdown();
    }
}
