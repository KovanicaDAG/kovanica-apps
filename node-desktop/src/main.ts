import { invoke } from "@tauri-apps/api/core";
import { fieldText, pathsReady, type Field } from "./format";

interface CheckedSetup {
  binary: string;
  binarySha256: string;
  configPath: string;
  dataDir: string;
  chainId: number;
  chainLabel: string;
  chainIdFromFile: boolean;
  rpcHttp: string;
  listenAddr: string;
}

interface ProcessStatus {
  state: "stopped" | "starting" | "running" | "stopping" | "exited";
  pid: number | null;
  exitCode: number | null;
  lastError: string | null;
  logs: string[];
}

interface Snapshot {
  process: ProcessStatus;
  monitor: {
    rpc: "unknown" | "reachable" | "unreachable";
    health: Record<string, Field>;
    sync: Record<string, Field>;
    stale: boolean;
    lastSuccessAgeMs: number | null;
    detail: string | null;
  };
  setup: CheckedSetup | null;
  reviewedNodeSource: string;
  stopGraceSecs: number;
  exitStopsOwnedChild: boolean;
}

const binaryInput = control<HTMLInputElement>("#binary-path");
const configInput = control<HTMLInputElement>("#config-path");
const dataInput = control<HTMLInputElement>("#data-path");
const checkButton = control<HTMLButtonElement>("#check-button");
const startButton = control<HTMLButtonElement>("#start-button");
const stopButton = control<HTMLButtonElement>("#stop-button");
const setupError = control<HTMLParagraphElement>("#setup-error");
const setupSummary = control<HTMLParagraphElement>("#setup-summary");
const processReadout = control<HTMLDListElement>("#process-readout");
const monitorReadout = control<HTMLDListElement>("#monitor-readout");
const logViewer = control<HTMLDivElement>("#log-viewer");

let checkedKey = "";
let pollTimer = 0;
let refreshInFlight = false;

function control<T extends Element>(selector: string): T {
  const found = document.querySelector<T>(selector);
  if (!found) throw new Error(`Node UI is missing ${selector}.`);
  return found;
}

function keyNow(): string {
  return `${binaryInput.value}\n${configInput.value}\n${dataInput.value}`;
}

function showError(message: string): void {
  setupError.hidden = false;
  setupError.textContent = message;
}

function clearError(): void {
  setupError.hidden = true;
  setupError.textContent = "";
}

function fill(list: HTMLDListElement, rows: Array<[string, string]>): void {
  list.replaceChildren();
  for (const [label, value] of rows) {
    const term = document.createElement("dt");
    term.textContent = label;
    const description = document.createElement("dd");
    description.textContent = value;
    list.append(term, description);
  }
}

function renderProcess(snapshot: Snapshot): void {
  const process = snapshot.process;
  const running = process.state === "running" || process.state === "starting";
  startButton.disabled = running || checkedKey === "" || checkedKey !== keyNow();
  stopButton.disabled = !running && process.state !== "stopping";
  fill(processReadout, [
    ["Process", process.state],
    ["PID", process.pid === null ? "none" : String(process.pid)],
    ["Exit code", process.exitCode === null ? "none" : String(process.exitCode)],
    ["Last error", process.lastError ?? "none"],
    ["Reviewed node source", snapshot.reviewedNodeSource],
    ["Stop grace", `${snapshot.stopGraceSecs}s SIGTERM, then SIGKILL`],
    ["On window close", snapshot.exitStopsOwnedChild ? "stops the node this app started" : "unknown"],
  ]);
  logViewer.replaceChildren();
  for (const line of process.logs) {
    const row = document.createElement("pre");
    row.textContent = line;
    logViewer.append(row);
  }
  logViewer.scrollTop = logViewer.scrollHeight;
}

function renderMonitor(snapshot: Snapshot): void {
  const monitor = snapshot.monitor;
  const health = monitor.health;
  const sync = snapshot.monitor.sync;
  const stale =
    monitor.stale && monitor.lastSuccessAgeMs !== null
      ? `stale last success ${Math.round(monitor.lastSuccessAgeMs / 1000)}s ago`
      : "current";
  fill(monitorReadout, [
    ["RPC", monitor.rpc],
    ["Freshness", stale],
    ["Monitor detail", monitor.detail ?? "none"],
    ["Health status", fieldText(health.status)],
    ["Health HTTP", fieldText(health.httpStatus)],
    ["Health flag", fieldText(health.healthy)],
    ["Health peer count", fieldText(health.peerCount)],
    ["Execution", fieldText(health.executionState)],
    ["Consensus", fieldText(health.consensusState)],
    ["Health error", fieldText(health.lastError)],
    ["Committed height", fieldText(sync.committedHeight)],
    ["Local tip round", fieldText(sync.localTipRound)],
    ["Catching up", fieldText(sync.catchingUp)],
    ["Sync peer count", fieldText(sync.peerCount)],
    ["Network tag", fieldText(sync.networkTag)],
    ["Peer ids", fieldText(sync.peerIds)],
  ]);
}

function renderSummary(setup: CheckedSetup): void {
  const source = setup.chainIdFromFile ? "from the config file" : "node default; chain_id is not in the file";
  setupSummary.textContent = [
    `${setup.chainLabel} (${source}).`,
    `RPC ${setup.rpcHttp}.`,
    `P2P listen ${setup.listenAddr} (shown, not rewritten).`,
    `Data ${setup.dataDir}.`,
    `Executable SHA-256 ${setup.binarySha256}.`,
    "The checksum identifies those bytes. It does not prove who built them.",
  ].join(" ");
}

async function refreshStatus(): Promise<void> {
  try {
    const snapshot = await invoke<Snapshot>("node_status");
    renderProcess(snapshot);
    renderMonitor(snapshot);
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
  }
}

async function refreshMonitor(): Promise<void> {
  if (refreshInFlight || document.hidden) return;
  refreshInFlight = true;
  try {
    const snapshot = await invoke<Snapshot>("monitor_refresh");
    renderProcess(snapshot);
    renderMonitor(snapshot);
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
  } finally {
    refreshInFlight = false;
  }
}

function invalidate(): void {
  if (checkedKey !== keyNow()) {
    startButton.disabled = true;
    setupSummary.textContent = "";
  }
}

for (const input of [binaryInput, configInput, dataInput]) {
  input.addEventListener("input", invalidate);
}

checkButton.addEventListener("click", async () => {
  clearError();
  if (!pathsReady(binaryInput.value, configInput.value, dataInput.value)) {
    showError("Enter the executable, config file, and data directory before checking.");
    checkButton.focus();
    return;
  }
  try {
    const setup = await invoke<CheckedSetup>("inspect_setup", {
      binary: binaryInput.value,
      configPath: configInput.value,
      dataDir: dataInput.value,
    });
    checkedKey = keyNow();
    renderSummary(setup);
    startButton.disabled = false;
    startButton.focus();
  } catch (error) {
    checkedKey = "";
    startButton.disabled = true;
    showError(error instanceof Error ? error.message : String(error));
  }
});

startButton.addEventListener("click", async () => {
  clearError();
  startButton.disabled = true;
  try {
    const snapshot = await invoke<Snapshot>("node_start", {
      binary: binaryInput.value,
      configPath: configInput.value,
      dataDir: dataInput.value,
    });
    renderProcess(snapshot);
    renderMonitor(snapshot);
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
    startButton.disabled = checkedKey === "" || checkedKey !== keyNow();
  }
});

stopButton.addEventListener("click", async () => {
  clearError();
  stopButton.disabled = true;
  try {
    const snapshot = await invoke<Snapshot>("node_stop");
    renderProcess(snapshot);
    renderMonitor(snapshot);
  } catch (error) {
    showError(error instanceof Error ? error.message : String(error));
  }
});

document.addEventListener("visibilitychange", () => {
  void invoke("set_monitor_paused", { paused: document.hidden }).catch(() => undefined);
  if (!document.hidden) void refreshMonitor();
});

pollTimer = window.setInterval(() => {
  void refreshStatus();
  if (!document.hidden) void refreshMonitor();
}, 2000);

window.addEventListener("beforeunload", () => {
  window.clearInterval(pollTimer);
});

void refreshStatus();
