import { freshnessText, fieldText, pathsReady, type Field } from "./format.ts";

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
  /** Bumps on each owned-process state change. Older invokes must not win. */
  statusSeq: number;
}

interface Monitor {
  rpc: "unknown" | "reachable" | "unreachable";
  health: Record<string, Field>;
  sync: Record<string, Field>;
  stale: boolean;
  lastSuccessAgeMs: number | null;
  detail: string | null;
}

interface Snapshot {
  process: ProcessStatus;
  monitor: Monitor;
  /** Bumps when the retained monitor view changes. */
  monitorSeq: number;
  setup: CheckedSetup | null;
  reviewedNodeSource: string;
  stopGraceSecs: number;
  exitStopsOwnedChild: boolean;
}

interface PathTuple {
  binary: string;
  config: string;
  data: string;
}

export interface Invoke {
  (command: string, args?: Record<string, unknown>): Promise<unknown>;
}

export interface AppHost {
  invoke: Invoke;
  document: {
    querySelector(selector: string): unknown;
    hidden: boolean;
    createElement(tag: string): {
      textContent: string;
    };
    addEventListener(type: string, listener: () => void): void;
    removeEventListener(type: string, listener: () => void): void;
  };
  window: {
    setInterval(handler: () => void, timeout: number): number;
    clearInterval(id: number): void;
    addEventListener(type: string, listener: () => void): void;
    removeEventListener(type: string, listener: () => void): void;
  };
}

interface TextEl {
  hidden: boolean;
  textContent: string;
}

interface InputEl {
  value: string;
  focus(): void;
  addEventListener(type: string, listener: () => void): void;
}

interface ButtonEl {
  disabled: boolean;
  focus(): void;
  addEventListener(type: string, listener: () => void): void;
}

interface ReadoutEl {
  replaceChildren(): void;
  append(...nodes: Array<{ textContent: string }>): void;
  scrollTop: number;
  scrollHeight: number;
  textContent: string;
}

function tupleKey(tuple: PathTuple): string {
  return `${tuple.binary}\n${tuple.config}\n${tuple.data}`;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Mount the node window. Returns a dispose function that drops the poll timer
 * and listeners so a reload cannot accumulate them.
 */
export function mountNodeApp(host: AppHost): () => void {
  const binaryInput = control<InputEl>("#binary-path");
  const configInput = control<InputEl>("#config-path");
  const dataInput = control<InputEl>("#data-path");
  const checkButton = control<ButtonEl>("#check-button");
  const startButton = control<ButtonEl>("#start-button");
  const stopButton = control<ButtonEl>("#stop-button");
  const setupError = control<TextEl>("#setup-error");
  const setupSummary = control<TextEl>("#setup-summary");
  const processReadout = control<ReadoutEl>("#process-readout");
  const monitorReadout = control<ReadoutEl>("#monitor-readout");
  const logViewer = control<ReadoutEl>("#log-viewer");

  let approvedKey = "";
  let approvedSummary = "";
  let inspectionEpoch = 0;
  let requestSeq = 0;
  let successBarrier = 0;
  let appliedProcessSeq = 0;
  let appliedMonitorSeq = 0;
  let lastState: ProcessStatus["state"] | null = null;
  let startEpoch = 0;
  let stopEpoch = 0;
  let startBusy = false;
  let stopBusy = false;
  let refreshInFlight = false;
  let monitorQueued = false;
  let pauseTicket = 0;
  let pauseSettled = 0;
  let pollTimer = 0;

  function control<T>(selector: string): T {
    const found = host.document.querySelector(selector);
    if (!found) throw new Error(`Node UI is missing ${selector}.`);
    return found as T;
  }

  function currentTuple(): PathTuple {
    return {
      binary: binaryInput.value,
      config: configInput.value,
      data: dataInput.value,
    };
  }

  function showError(message: string): void {
    setupError.hidden = false;
    setupError.textContent = message;
  }

  function clearError(): void {
    setupError.hidden = true;
    setupError.textContent = "";
  }

  function issueRequest(): number {
    requestSeq += 1;
    return requestSeq;
  }

  function markSuccess(id: number): void {
    if (id > successBarrier) successBarrier = id;
  }

  function reportError(id: number, error: unknown): void {
    if (id < successBarrier) return;
    showError(errorText(error));
  }

  function fill(list: ReadoutEl, rows: Array<[string, string]>): void {
    list.replaceChildren();
    for (const [label, value] of rows) {
      const term = host.document.createElement("dt");
      term.textContent = label;
      const description = host.document.createElement("dd");
      description.textContent = value;
      list.append(term, description);
    }
  }

  function syncButtons(): void {
    const live =
      lastState === "running" || lastState === "starting" || lastState === "stopping";
    const approved = approvedKey !== "" && approvedKey === tupleKey(currentTuple());
    startButton.disabled = startBusy || live || !approved;
    stopButton.disabled = stopBusy || !live;
  }

  function syncApproval(): void {
    const showing = approvedKey !== "" && approvedKey === tupleKey(currentTuple());
    setupSummary.textContent = showing ? approvedSummary : "";
    syncButtons();
  }

  function renderLogs(snapshot: Snapshot): void {
    logViewer.replaceChildren();
    for (const line of snapshot.process.logs) {
      const row = host.document.createElement("pre");
      row.textContent = line;
      logViewer.append(row);
    }
    logViewer.scrollTop = logViewer.scrollHeight;
  }

  function renderProcess(snapshot: Snapshot): void {
    const process = snapshot.process;
    lastState = process.state;
    fill(processReadout, [
      ["Process", process.state],
      ["PID", process.pid === null ? "none" : String(process.pid)],
      ["Exit code", process.exitCode === null ? "none" : String(process.exitCode)],
      ["Last error", process.lastError ?? "none"],
      ["Reviewed node source", snapshot.reviewedNodeSource],
      ["Stop grace", `${snapshot.stopGraceSecs}s SIGTERM, then SIGKILL`],
      [
        "On window close",
        snapshot.exitStopsOwnedChild
          ? "same Stop; stays open if the child is not confirmed exited"
          : "unknown",
      ],
    ]);
    renderLogs(snapshot);
    syncButtons();
  }

  function renderMonitor(snapshot: Snapshot): void {
    const monitor = snapshot.monitor;
    const health = monitor.health;
    const sync = monitor.sync;
    fill(monitorReadout, [
      ["RPC", monitor.rpc],
      ["Freshness", freshnessText(monitor)],
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

  function summaryText(setup: CheckedSetup): string {
    const source = setup.chainIdFromFile
      ? "from the config file"
      : "node default; chain_id is not in the file";
    return [
      `${setup.chainLabel} (${source}).`,
      `RPC ${setup.rpcHttp}.`,
      `P2P listen ${setup.listenAddr} (shown, not rewritten).`,
      `Data ${setup.dataDir}.`,
      `Executable SHA-256 ${setup.binarySha256}.`,
      "The checksum identifies those bytes. It does not prove who built them.",
    ].join(" ");
  }

  /**
   * Apply a snapshot only when its backend sequence is newer.
   * A newer process observation also supplies the monitor view captured with it,
   * so a late healthy read cannot repaint a stopped or replaced process.
   * Equal process sequence still accepts a newer monitor and refreshes logs.
   */
  function applySnapshot(snapshot: Snapshot): boolean {
    let applied = false;
    if (snapshot.process.statusSeq > appliedProcessSeq) {
      appliedProcessSeq = snapshot.process.statusSeq;
      renderProcess(snapshot);
      renderMonitor(snapshot);
      if (snapshot.monitorSeq > appliedMonitorSeq) {
        appliedMonitorSeq = snapshot.monitorSeq;
      }
      applied = true;
    } else if (snapshot.process.statusSeq === appliedProcessSeq) {
      renderLogs(snapshot);
      if (snapshot.monitorSeq > appliedMonitorSeq) {
        appliedMonitorSeq = snapshot.monitorSeq;
        renderMonitor(snapshot);
        applied = true;
      }
    }
    return applied;
  }

  async function refreshStatus(): Promise<void> {
    const id = issueRequest();
    try {
      const snapshot = (await host.invoke("node_status")) as Snapshot;
      if (applySnapshot(snapshot)) markSuccess(id);
    } catch (error) {
      reportError(id, error);
    }
  }

  async function refreshMonitor(): Promise<void> {
    if (host.document.hidden) return;
    if (refreshInFlight) {
      monitorQueued = true;
      return;
    }
    refreshInFlight = true;
    const id = issueRequest();
    try {
      const snapshot = (await host.invoke("monitor_refresh")) as Snapshot;
      if (applySnapshot(snapshot)) markSuccess(id);
    } catch (error) {
      reportError(id, error);
    } finally {
      refreshInFlight = false;
      if (monitorQueued) {
        monitorQueued = false;
        if (!host.document.hidden) void refreshMonitor();
      }
    }
  }

  async function syncPaused(): Promise<void> {
    const hidden = host.document.hidden;
    const ticket = ++pauseTicket;
    const id = issueRequest();
    try {
      await host.invoke("set_monitor_paused", { paused: hidden });
      if (ticket < pauseSettled) return;
      pauseSettled = ticket;
    } catch (error) {
      if (ticket < pauseSettled) return;
      pauseSettled = ticket;
      reportError(id, error);
      return;
    }
    if (ticket === pauseTicket && !host.document.hidden) void refreshMonitor();
  }

  function onInput(): void {
    inspectionEpoch += 1;
    syncApproval();
  }

  async function onCheck(): Promise<void> {
    const epoch = ++inspectionEpoch;
    const requested = currentTuple();
    const id = issueRequest();
    if (!pathsReady(requested.binary, requested.config, requested.data)) {
      approvedKey = "";
      approvedSummary = "";
      syncApproval();
      if (epoch === inspectionEpoch) {
        showError("Enter the executable, config file, and data directory before checking.");
        checkButton.focus();
      }
      return;
    }
    approvedKey = "";
    approvedSummary = "";
    syncApproval();
    try {
      const setup = (await host.invoke("inspect_setup", {
        binary: requested.binary,
        configPath: requested.config,
        dataDir: requested.data,
      })) as CheckedSetup;
      if (epoch !== inspectionEpoch) return;
      if (tupleKey(requested) !== tupleKey(currentTuple())) return;
      approvedKey = tupleKey(requested);
      approvedSummary = summaryText(setup);
      markSuccess(id);
      clearError();
      syncApproval();
      startButton.focus();
    } catch (error) {
      if (epoch !== inspectionEpoch) return;
      if (tupleKey(requested) !== tupleKey(currentTuple())) return;
      approvedKey = "";
      approvedSummary = "";
      syncApproval();
      reportError(id, error);
    }
  }

  async function onStart(): Promise<void> {
    if (startButton.disabled || startBusy) return;
    const epoch = ++startEpoch;
    const requested = currentTuple();
    if (tupleKey(requested) !== approvedKey) return;
    const id = issueRequest();
    startBusy = true;
    syncButtons();
    try {
      const snapshot = (await host.invoke("node_start", {
        binary: requested.binary,
        configPath: requested.config,
        dataDir: requested.data,
      })) as Snapshot;
      if (epoch !== startEpoch) return;
      if (applySnapshot(snapshot)) {
        markSuccess(id);
        clearError();
      }
    } catch (error) {
      if (epoch !== startEpoch) return;
      reportError(id, error);
    } finally {
      if (epoch === startEpoch) startBusy = false;
      syncButtons();
    }
  }

  async function onStop(): Promise<void> {
    if (stopButton.disabled || stopBusy) return;
    const epoch = ++stopEpoch;
    const id = issueRequest();
    stopBusy = true;
    syncButtons();
    try {
      const snapshot = (await host.invoke("node_stop")) as Snapshot;
      if (epoch !== stopEpoch) return;
      if (applySnapshot(snapshot)) {
        markSuccess(id);
        clearError();
      }
    } catch (error) {
      if (epoch !== stopEpoch) return;
      reportError(id, error);
    } finally {
      if (epoch === stopEpoch) stopBusy = false;
      syncButtons();
    }
  }

  function onVisibility(): void {
    void syncPaused();
  }

  function onPoll(): void {
    void refreshStatus();
    if (!host.document.hidden) void refreshMonitor();
  }

  function onUnload(): void {
    host.window.clearInterval(pollTimer);
    pollTimer = 0;
  }

  for (const input of [binaryInput, configInput, dataInput]) {
    input.addEventListener("input", onInput);
  }
  checkButton.addEventListener("click", () => {
    void onCheck();
  });
  startButton.addEventListener("click", () => {
    void onStart();
  });
  stopButton.addEventListener("click", () => {
    void onStop();
  });
  host.document.addEventListener("visibilitychange", onVisibility);
  host.window.addEventListener("beforeunload", onUnload);
  pollTimer = host.window.setInterval(onPoll, 2000);
  void refreshStatus();
  void syncPaused();

  return () => {
    onUnload();
    host.document.removeEventListener("visibilitychange", onVisibility);
    host.window.removeEventListener("beforeunload", onUnload);
  };
}
