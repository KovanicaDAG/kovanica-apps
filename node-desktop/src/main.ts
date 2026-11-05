import { invoke } from "@tauri-apps/api/core";

/* ---------------------- DOM elements ---------------------- */
function control<T extends Element>(selector: string): T {
  const found = document.querySelector<T>(selector);
  if (!found) throw new Error(`Node-desktop UI is missing ${selector}.`);
  return found;
}

const startBtn = control<HTMLButtonElement>("#start-button");
const stopBtn = control<HTMLButtonElement>("#stop-button");
const validatorToggle = control<HTMLInputElement>("#validator-toggle");
const validatorStatus = control<HTMLParagraphElement>("#validator-status");
const stakeBtn = control<HTMLButtonElement>("#stake-button");
const keepRunning = control<HTMLInputElement>("#keep-running");
const logViewer = control<HTMLDivElement>("#log-viewer");
const statusDl = control<HTMLDListElement>("#status");
const validatorStakeValue = control<HTMLSpanElement>("#validator-stake-value");
const validatorShortfall = control<HTMLSpanElement>("#validator-shortfall");
const validatorShortfallValue = control<HTMLSpanElement>("#validator-shortfall-value");

/* ---------------------- DTOs ---------------------- */
interface StatusDto {
  state: string;
  pid: number | null;
  network_tag: string | null;
  expected_tag: string;
  chain_id: number;
  committed_height: number | null;
  local_round: number | null;
  peer_count: number | null;
  catching_up: boolean | null;
  rpc_url: string;
  data_dir: string;
  config_path: string;
  last_error: string | null;
}

interface ValidatorDto {
  run_validator: boolean;
  has_key: boolean;
  key_address: string | null;
  stake_atoms: string | null;
  stake_kuna: string | null;
  min_stake_atoms: string;
  min_stake_kuna: string;
  eligible: boolean;
  can_toggle: boolean;
  stake_available: boolean;
  message: string | null;
}

interface ActionResult {
  ok: boolean;
  message: string | null;
  tx_id: string | null;
}

interface SettingsDto {
  keep_running: boolean;
}

/* ---------------------- Helpers ---------------------- */
function showError(message: string): void {
  console.error(message);
}

function clearNode(): void {
  statusDl.replaceChildren();
}

function fillRows(rows: { label: string; value: string }[]): void {
  statusDl.replaceChildren();
  for (const row of rows) {
    const term = document.createElement("dt");
    term.textContent = row.label;
    const value = document.createElement("dd");
    value.textContent = row.value;
    statusDl.append(term, value);
  }
}

function addLogLine(line: string): void {
  const pre = document.createElement("pre");
  pre.textContent = line;
  logViewer.appendChild(pre);
  const children = logViewer.children;
  while (children.length > 200) {
    logViewer.removeChild(children[0]);
  }
}

let currentStatus: StatusDto | null = null;

async function loadStatus(): Promise<void> {
  try {
    const status = await invoke<StatusDto>("node_status");
    currentStatus = status;
    if (status.state !== "running") {
      updateUIStopped(status);
      return;
    }
    updateUIRunning(status);
  } catch (err) {
    showError(`Failed to load status: ${err}`);
    updateUIStopped(null);
  }
}

function updateUIStopped(_status: StatusDto | null): void {
  startBtn.disabled = false;
  stopBtn.disabled = true;
  validatorToggle.disabled = true;
  validatorToggle.checked = false;
  stakeBtn.disabled = true;
  stakeBtn.textContent = "Stake (add missing KUNA)";
  validatorStatus.hidden = true;
  clearNode();
  if (_status) {
    fillRows([
      { label: "state", value: _status.state },
      { label: "expected tag", value: _status.expected_tag },
    ]);
  }
}

function updateUIRunning(status: StatusDto): void {
  startBtn.disabled = true;
  stopBtn.disabled = false;

  fillRows([
    { label: "state", value: status.state },
    { label: "pid", value: status.pid !== null ? String(status.pid) : "—" },
    { label: "chain id", value: String(status.chain_id) },
    { label: "network tag", value: status.network_tag ?? "—" },
    { label: "committed height", value: status.committed_height !== null ? String(status.committed_height) : "—" },
    { label: "local round", value: status.local_round !== null ? String(status.local_round) : "—" },
    { label: "peer count", value: status.peer_count !== null ? String(status.peer_count) : "—" },
    { label: "catching up", value: status.catching_up !== null ? String(status.catching_up) : "—" },
    { label: "rpc url", value: status.rpc_url },
  ]);

  void loadValidator();
}

async function loadValidator(): Promise<void> {
  try {
    const v = await invoke<ValidatorDto>("validator_state");
    applyValidatorUI(v);
  } catch (err) {
    showError(`Failed to load validator state: ${err}`);
  }
}

function applyValidatorUI(v: ValidatorDto): void {
  validatorToggle.disabled = !v.can_toggle;
  validatorToggle.checked = v.run_validator;
  validatorToggle.indeterminate = false;

  if (v.run_validator) {
    validatorStatus.hidden = false;
    if (v.stake_kuna !== null) {
      validatorStakeValue.textContent = v.stake_kuna;
    }
    stakeBtn.disabled = !v.stake_available;
    stakeBtn.textContent = "Running as validator";
  } else {
    validatorStatus.hidden = true;
    stakeBtn.disabled = !v.stake_available;
    stakeBtn.textContent = "Stake (add missing KUNA)";
  }

  if (v.stake_atoms !== null) {
    const stakeNum = Number(v.stake_kuna ?? "0");
    if (stakeNum < 50000) {
      validatorShortfall.hidden = false;
      validatorShortfallValue.textContent = String(50000 - Math.floor(stakeNum));
      stakeBtn.disabled = true;
      stakeBtn.textContent = "Stake (add missing KUNA)";
    } else {
      validatorShortfall.hidden = true;
    }
  } else {
    validatorShortfall.hidden = true;
  }

  if (v.message) {
    showError(v.message);
  }
}

async function loadSettings(): Promise<void> {
  try {
    const s = await invoke<SettingsDto>("settings_get");
    keepRunning.checked = s.keep_running;
  } catch (err) {
    showError(`Failed to load settings: ${err}`);
  }
}

async function loadLogs(): Promise<void> {
  try {
    const logs = await invoke<string[]>("node_logs");
    // only append new tail
    logViewer.innerHTML = "";
    for (const line of logs) {
      addLogLine(line);
    }
  } catch (err) {
    showError(`Failed to load logs: ${err}`);
  }
}

/* ---------------------- Event listeners ---------------------- */
startBtn.addEventListener("click", async () => {
  startBtn.disabled = true;
  startBtn.textContent = "Starting...";
  try {
    await invoke("node_start");
    await loadStatus();
  } catch (err) {
    showError(`Failed to start: ${err}`);
  } finally {
    startBtn.disabled = false;
    startBtn.textContent = "Start";
  }
});

stopBtn.addEventListener("click", async () => {
  stopBtn.disabled = true;
  stopBtn.textContent = "Stopping...";
  try {
    await invoke("node_stop");
    await loadStatus();
  } catch (err) {
    showError(`Failed to stop: ${err}`);
  } finally {
    stopBtn.disabled = false;
    stopBtn.textContent = "Stop";
  }
});

validatorToggle.addEventListener("change", async () => {
  const desired = validatorToggle.checked;
  validatorToggle.disabled = true;
  validatorToggle.indeterminate = true;
  try {
    await invoke("set_run_validator", { enabled: desired });
    await loadValidator();
  } catch (err) {
    showError(`Failed to toggle validator: ${err}`);
    validatorToggle.checked = !desired;
  } finally {
    validatorToggle.disabled = false;
    validatorToggle.indeterminate = false;
  }
});

stakeBtn.addEventListener("click", async () => {
  const amount = window.prompt("Amount in KUNA to stake:") ?? "";
  if (!amount) return;
  stakeBtn.disabled = true;
  stakeBtn.textContent = "Staking...";
  try {
    const res = await invoke<ActionResult>("stake", { amount_kuna: amount });
    if (!res.ok) {
      showError(res.message ?? "Stake failed");
    }
    await loadValidator();
  } catch (err) {
    showError(`Stake failed: ${err}`);
  } finally {
    stakeBtn.disabled = false;
    stakeBtn.textContent = "Stake (add missing KUNA)";
  }
});

keepRunning.addEventListener("change", async () => {
  try {
    await invoke("set_keep_running", { enabled: keepRunning.checked });
  } catch (err) {
    showError(`Failed to save settings: ${err}`);
  }
});

/* ---------------------- Polling ---------------------- */
async function poll(): Promise<void> {
  await loadStatus();
  if (currentStatus && currentStatus.state === "running") {
    await loadLogs();
  }
}

void (async () => {
  await loadSettings();
  await poll();
  window.setInterval(() => {
    void poll();
  }, 1500);
})();
