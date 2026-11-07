import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { fieldText, freshnessText, pathsReady } from "../src/format.ts";
import { mountNodeApp } from "../src/ui.ts";

// Mocked invoke and a fake document. Not a kovanica-node process and not a GTK window.
// The handlers under test are the ones `src/main.ts` mounts in the real window.

const root = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(join(root, "../index.html"), "utf8");
const css = readFileSync(join(root, "../src/styles.css"), "utf8");
const main = readFileSync(join(root, "../src/main.ts"), "utf8");
const ui = readFileSync(join(root, "../src/ui.ts"), "utf8");

test("missing monitor values are not shown as zero or synced", () => {
  assert.equal(fieldText({ kind: "unknown" }), "unknown");
  assert.equal(fieldText({ kind: "unavailable" }), "unavailable");
  assert.equal(fieldText({ kind: "value", value: "0" }), "0");
  assert.equal(fieldText({ kind: "value", value: null }), "unknown");
  assert.equal(pathsReady("", "/cfg", "/data"), false);
  assert.equal(pathsReady("/bin", "/cfg", "/data"), true);
  assert.equal(freshnessText({ rpc: "unknown", stale: false, lastSuccessAgeMs: null }), "not observed");
  assert.equal(freshnessText({ rpc: "unreachable", stale: false, lastSuccessAgeMs: null }), "unreachable");
  assert.equal(freshnessText({ rpc: "reachable", stale: false, lastSuccessAgeMs: 0 }), "current");
  assert.equal(
    freshnessText({ rpc: "unreachable", stale: true, lastSuccessAgeMs: 4000 }),
    "stale last success 4s ago",
  );
  assert.equal(freshnessText({ rpc: "reachable", stale: true, lastSuccessAgeMs: null }), "stale; age unknown");
  assert.notEqual(freshnessText({ rpc: "unknown", stale: false, lastSuccessAgeMs: null }), "current");
});

test("the window has process controls and no wallet, stake, or mining controls", () => {
  assert.match(html, /id="dev-warning"/);
  assert.match(html, /role="alert"/);
  assert.match(html, /id="start-button"/);
  assert.match(html, /id="stop-button"/);
  assert.match(html, /id="log-viewer"/);
  assert.match(html, /tabindex="0"/);
  assert.match(html, /id="process-readout"[^>]*aria-live="polite"/);
  assert.match(html, /id="monitor-readout"[^>]*aria-live="polite"/);
  assert.match(html, /id="setup-summary"[^>]*aria-live="polite"/);
  assert.doesNotMatch(html, /id="(validator-toggle|stake-button|mnemonic|seed-field)"/);
  assert.doesNotMatch(html, /type="password"/);
  assert.doesNotMatch(main, /wallet_create|sign_transfer|stake-button|run_validator|mnemonic/);
  assert.doesNotMatch(ui, /wallet_create|sign_transfer|stake-button|run_validator|mnemonic/);
  assert.match(css, /:focus-visible/);
  assert.match(ui, /document\.hidden/);
  assert.match(ui, /refreshInFlight/);
  assert.match(main, /mountNodeApp/);
});

interface Pending {
  command: string;
  args?: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
}

interface FakeEl {
  id: string;
  tag: string;
  value: string;
  disabled: boolean;
  hidden: boolean;
  textContent: string;
  children: FakeEl[];
  focused: boolean;
  scrollTop: number;
  scrollHeight: number;
  listeners: Record<string, Array<() => void>>;
  addEventListener(type: string, listener: () => void): void;
  dispatch(type: string): void;
  press(key: string): void;
  focus(): void;
  replaceChildren(): void;
  append(...nodes: FakeEl[]): void;
}

function element(tag: string, id = ""): FakeEl {
  const el: FakeEl = {
    id,
    tag,
    value: "",
    disabled: tag === "button" ? false : false,
    hidden: false,
    textContent: "",
    children: [],
    focused: false,
    scrollTop: 0,
    scrollHeight: 0,
    listeners: {},
    addEventListener(type, listener) {
      (el.listeners[type] ??= []).push(listener);
    },
    dispatch(type) {
      for (const listener of el.listeners[type] ?? []) listener();
    },
    press(key) {
      if (el.disabled) return;
      if (el.tag === "button" && (key === "Enter" || key === " ")) el.dispatch("click");
    },
    focus() {
      el.focused = true;
    },
    replaceChildren() {
      el.children = [];
    },
    append(...nodes) {
      el.children.push(...nodes);
    },
  };
  return el;
}

function readout(el: FakeEl): Record<string, string> {
  const rows: Record<string, string> = {};
  for (let index = 0; index < el.children.length; index += 2) {
    rows[el.children[index].textContent] = el.children[index + 1].textContent;
  }
  return rows;
}

function field(kind: "value" | "unknown" | "unavailable", value?: string) {
  return { kind, value: value ?? null };
}

function blankFields(kind: "unknown" | "unavailable") {
  return {
    status: field(kind),
    httpStatus: field(kind),
    healthy: field(kind),
    peerCount: field(kind),
    executionState: field(kind),
    consensusState: field(kind),
    lastError: field(kind),
  };
}

function blankSync(kind: "unknown" | "unavailable") {
  return {
    committedHeight: field(kind),
    localTipRound: field(kind),
    catchingUp: field(kind),
    peerCount: field(kind),
    networkTag: field(kind),
    peerIds: field(kind),
  };
}

function snapshot(options: {
  state?: "stopped" | "starting" | "running" | "stopping" | "exited";
  statusSeq: number;
  monitorSeq: number;
  rpc?: "unknown" | "reachable" | "unreachable";
  stale?: boolean;
  lastSuccessAgeMs?: number | null;
  detail?: string | null;
  height?: string | null;
  heightKind?: "value" | "unknown" | "unavailable";
  catchingUp?: string | null;
  peerCount?: string | null;
  logs?: string[];
  pid?: number | null;
  lastError?: string | null;
}) {
  const kind = options.rpc === "unreachable" && options.lastSuccessAgeMs == null ? "unavailable" : "unknown";
  const sync = blankSync(options.heightKind === "unavailable" ? "unavailable" : kind === "unavailable" && !options.height ? "unavailable" : "unknown");
  if (options.height !== undefined && options.height !== null) {
    sync.committedHeight = field(options.heightKind ?? "value", options.height);
  } else if (options.heightKind) {
    sync.committedHeight = field(options.heightKind);
  }
  if (options.catchingUp !== undefined && options.catchingUp !== null) {
    sync.catchingUp = field("value", options.catchingUp);
  }
  if (options.peerCount !== undefined && options.peerCount !== null) {
    sync.peerCount = field("value", options.peerCount);
  }
  const healthKind = options.rpc === "reachable" ? "unknown" : kind;
  return {
    process: {
      state: options.state ?? "stopped",
      pid: options.pid ?? null,
      exitCode: options.state === "exited" ? 0 : null,
      lastError: options.lastError ?? null,
      logs: options.logs ?? [],
      statusSeq: options.statusSeq,
    },
    monitor: {
      rpc: options.rpc ?? "unknown",
      health: options.rpc === "reachable"
        ? { ...blankFields("unknown"), status: field("value", "ok"), healthy: field("value", "true") }
        : blankFields(healthKind === "unavailable" ? "unavailable" : "unknown"),
      sync,
      stale: options.stale ?? false,
      lastSuccessAgeMs: options.lastSuccessAgeMs ?? null,
      detail: options.detail ?? null,
    },
    monitorSeq: options.monitorSeq,
    setup: null,
    reviewedNodeSource: "74045cf3e27a70569c32af35aec24ada5dd9ff79",
    stopGraceSecs: 10,
    exitStopsOwnedChild: true,
  };
}

function checked(tuple: { binary: string; config: string; data: string }, sha = "abc") {
  return {
    binary: tuple.binary,
    binarySha256: sha,
    configPath: tuple.config,
    dataDir: tuple.data,
    chainId: 1337,
    chainLabel: "local (chain 1337)",
    chainIdFromFile: true,
    rpcHttp: "http://127.0.0.1:8545",
    listenAddr: "127.0.0.1:8000",
  };
}

function harness() {
  const ids = ["binary-path", "config-path", "data-path", "check-button", "start-button", "stop-button", "setup-error", "setup-summary", "process-readout", "monitor-readout", "log-viewer"];
  const tags: Record<string, string> = {
    "binary-path": "input",
    "config-path": "input",
    "data-path": "input",
    "check-button": "button",
    "start-button": "button",
    "stop-button": "button",
    "setup-error": "p",
    "setup-summary": "p",
    "process-readout": "dl",
    "monitor-readout": "dl",
    "log-viewer": "div",
  };
  const nodes = new Map(ids.map((id) => [id, element(tags[id], id)]));
  nodes.get("start-button")!.disabled = true;
  nodes.get("stop-button")!.disabled = true;
  nodes.get("setup-error")!.hidden = true;
  const listeners: Record<string, Array<() => void>> = {};
  const timers = new Map<number, () => void>();
  let nextTimer = 1;
  const pending: Pending[] = [];
  const document = {
    hidden: false,
    querySelector(selector: string) {
      return nodes.get(selector.slice(1)) ?? null;
    },
    createElement(tag: string) {
      return element(tag);
    },
    addEventListener(type: string, listener: () => void) {
      (listeners[type] ??= []).push(listener);
    },
    removeEventListener(type: string, listener: () => void) {
      listeners[type] = (listeners[type] ?? []).filter((item) => item !== listener);
    },
    dispatch(type: string) {
      for (const listener of listeners[type] ?? []) listener();
    },
  };
  const window = {
    setInterval(handler: () => void) {
      const id = nextTimer++;
      timers.set(id, handler);
      return id;
    },
    clearInterval(id: number) {
      timers.delete(id);
    },
    addEventListener(type: string, listener: () => void) {
      (listeners[type] ??= []).push(listener);
    },
    removeEventListener(type: string, listener: () => void) {
      listeners[type] = (listeners[type] ?? []).filter((item) => item !== listener);
    },
    dispatch(type: string) {
      for (const listener of listeners[type] ?? []) listener();
    },
  };
  const invoke = (command: string, args?: Record<string, unknown>) =>
    new Promise((resolve, reject) => {
      pending.push({ command, args, resolve, reject });
    });
  const dispose = mountNodeApp({ invoke, document, window });
  function take(command: string): Pending {
    const index = pending.findIndex((item) => item.command === command);
    assert.notEqual(index, -1, `missing invoke ${command}; have ${pending.map((item) => item.command).join(",")}`);
    return pending.splice(index, 1)[0];
  }
  async function flush() {
    for (let step = 0; step < 8; step += 1) await Promise.resolve();
  }
  return { nodes, document, window, pending, timers, dispose, take, flush, invoke };
}

async function boot(world: ReturnType<typeof harness>, snap = snapshot({ statusSeq: 1, monitorSeq: 1 })) {
  const status = world.take("node_status");
  const pause = world.take("set_monitor_paused");
  assert.equal(pause.args?.paused, false);
  status.resolve(snap);
  pause.resolve(null);
  await world.flush();
  const refresh = world.take("monitor_refresh");
  refresh.resolve(snap);
  await world.flush();
}

function fillPaths(world: ReturnType<typeof harness>, tuple = { binary: "/bin/node", config: "/cfg/node.toml", data: "/data/node" }) {
  world.nodes.get("binary-path")!.value = tuple.binary;
  world.nodes.get("config-path")!.value = tuple.config;
  world.nodes.get("data-path")!.value = tuple.data;
  return tuple;
}

test("delayed check results approve only the tuple that was submitted", async () => {
  const world = harness();
  await boot(world);
  const first = { binary: "/bin/a", config: "/cfg/a.toml", data: "/data/a" };
  const second = { binary: "/bin/a", config: "/cfg/a.toml", data: "/data/b" };
  fillPaths(world, first);
  world.nodes.get("check-button")!.press("Enter");
  const pendingA = world.take("inspect_setup");
  assert.deepEqual(pendingA.args, { binary: first.binary, configPath: first.config, dataDir: first.data });
  world.nodes.get("data-path")!.value = second.data;
  world.nodes.get("data-path")!.dispatch("input");
  pendingA.resolve(checked(second, "should-not-apply"));
  await world.flush();
  assert.equal(world.nodes.get("start-button")!.disabled, true);
  assert.equal(world.nodes.get("setup-summary")!.textContent, "");

  world.nodes.get("check-button")!.press(" ");
  const pendingB = world.take("inspect_setup");
  assert.equal(pendingB.args?.dataDir, second.data);
  world.nodes.get("data-path")!.value = first.data;
  world.nodes.get("data-path")!.dispatch("input");
  world.nodes.get("check-button")!.dispatch("click");
  const pendingC = world.take("inspect_setup");
  assert.equal(pendingC.args?.dataDir, first.data);
  pendingB.resolve(checked(second, "late-old"));
  await world.flush();
  assert.equal(world.nodes.get("start-button")!.disabled, true);
  assert.equal(world.nodes.get("setup-summary")!.textContent, "");
  pendingC.resolve(checked(first, "fresh-sha"));
  await world.flush();
  assert.equal(world.nodes.get("start-button")!.disabled, false);
  assert.match(world.nodes.get("setup-summary")!.textContent, /fresh-sha/);
  assert.match(world.nodes.get("setup-summary")!.textContent, /\/data\/a/);
  assert.doesNotMatch(world.nodes.get("setup-summary")!.textContent, /late-old/);
  assert.equal(world.nodes.get("start-button")!.focused, true);
  world.dispose();
});

test("a failed inspection does not clear a newer acceptance or approve after editing", async () => {
  const world = harness();
  await boot(world);
  const tuple = fillPaths(world);
  world.nodes.get("check-button")!.dispatch("click");
  world.take("inspect_setup").resolve(checked(tuple, "ok-sha"));
  await world.flush();
  assert.equal(world.nodes.get("start-button")!.disabled, false);

  world.nodes.get("data-path")!.value = "/data/other";
  world.nodes.get("data-path")!.dispatch("input");
  assert.equal(world.nodes.get("start-button")!.disabled, true);
  assert.equal(world.nodes.get("setup-summary")!.textContent, "");
  world.nodes.get("data-path")!.value = tuple.data;
  world.nodes.get("data-path")!.dispatch("input");
  assert.match(world.nodes.get("setup-summary")!.textContent, /ok-sha/);
  assert.equal(world.nodes.get("start-button")!.disabled, false);

  world.nodes.get("check-button")!.dispatch("click");
  const olderFailure = world.take("inspect_setup");
  assert.equal(world.nodes.get("start-button")!.disabled, true);
  world.nodes.get("check-button")!.dispatch("click");
  const newer = world.take("inspect_setup");
  newer.resolve(checked(tuple, "again-sha"));
  await world.flush();
  olderFailure.reject(new Error("config was rejected"));
  await world.flush();
  assert.equal(world.nodes.get("setup-error")!.hidden, true);
  assert.match(world.nodes.get("setup-summary")!.textContent, /again-sha/);
  assert.equal(world.nodes.get("start-button")!.disabled, false);

  world.nodes.get("check-button")!.dispatch("click");
  world.take("inspect_setup").reject("chain id 1 is mainnet");
  await world.flush();
  assert.equal(world.nodes.get("start-button")!.disabled, true);
  assert.equal(world.nodes.get("setup-summary")!.textContent, "");
  assert.equal(world.nodes.get("setup-error")!.hidden, false);
  assert.match(world.nodes.get("setup-error")!.textContent, /mainnet/);
  world.dispose();
});

test("older snapshots and late errors do not overwrite a newer lifecycle", async () => {
  const world = harness();
  const running = snapshot({
    state: "running",
    statusSeq: 2,
    monitorSeq: 3,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "4",
    catchingUp: "false",
    pid: 44,
    logs: ["started"],
  });
  await boot(world, running);
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "running");
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "current");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "4");
  assert.doesNotMatch(world.nodes.get("monitor-readout")!.children.map((child) => child.textContent).join(" "), /synced|recovered/i);

  world.timers.get([...world.timers.keys()][0])!();
  const staleStatus = world.take("node_status");
  const staleMonitor = world.take("monitor_refresh");
  world.nodes.get("stop-button")!.press("Enter");
  const stop = world.take("node_stop");
  const exited = snapshot({
    state: "exited",
    statusSeq: 4,
    monitorSeq: 4,
    rpc: "unreachable",
    detail: "owned process exited with code 0; monitor is not live",
    heightKind: "unavailable",
    logs: ["started", "exited code 0"],
  });
  stop.resolve(exited);
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "exited");
  assert.equal(world.nodes.get("stop-button")!.disabled, true);
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "unreachable");

  staleStatus.resolve(snapshot({ state: "running", statusSeq: 2, monitorSeq: 8, rpc: "reachable", lastSuccessAgeMs: 0, height: "9", catchingUp: "false", pid: 44 }));
  staleMonitor.resolve(snapshot({ state: "running", statusSeq: 2, monitorSeq: 9, rpc: "reachable", lastSuccessAgeMs: 0, height: "9", catchingUp: "false", pid: 44 }));
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "exited");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "unavailable");
  assert.notEqual(readout(world.nodes.get("monitor-readout")!).Freshness, "current");
  assert.equal(world.nodes.get("start-button")!.disabled, true);

  world.timers.get([...world.timers.keys()][0])!();
  const lateError = world.take("node_status");
  const newer = world.take("monitor_refresh");
  newer.resolve(snapshot({
    state: "exited",
    statusSeq: 4,
    monitorSeq: 5,
    rpc: "unreachable",
    detail: "owned process exited with code 0; monitor is not live",
    heightKind: "unavailable",
  }));
  await world.flush();
  lateError.reject(new Error("status failed after exit"));
  await world.flush();
  assert.equal(world.nodes.get("setup-error")!.hidden, true);
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "exited");
  world.dispose();
});

test("monitor states stay distinct across first failure, outage, and recovery", async () => {
  const world = harness();
  await boot(world, snapshot({ statusSeq: 1, monitorSeq: 1 }));
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "not observed");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "unknown");
  assert.notEqual(readout(world.nodes.get("monitor-readout")!)["Committed height"], "0");

  world.timers.get([...world.timers.keys()][0])!();
  world.take("node_status").resolve(snapshot({
    state: "running",
    statusSeq: 2,
    monitorSeq: 1,
    pid: 7,
  }));
  await world.flush();
  const firstFailure = world.take("monitor_refresh");
  firstFailure.resolve(snapshot({
    state: "running",
    statusSeq: 2,
    monitorSeq: 2,
    rpc: "unreachable",
    detail: "health request failed: timed out",
    heightKind: "unavailable",
    pid: 7,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "unreachable");
  assert.equal(readout(world.nodes.get("monitor-readout")!).RPC, "unreachable");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "unavailable");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Health flag"], "unavailable");
  assert.match(readout(world.nodes.get("monitor-readout")!)["Monitor detail"], /timed out/);

  world.timers.get([...world.timers.keys()][0])!();
  world.take("node_status").resolve(snapshot({ state: "running", statusSeq: 2, monitorSeq: 2, pid: 7, logs: ["line"] }));
  const success = world.take("monitor_refresh");
  success.resolve(snapshot({
    state: "running",
    statusSeq: 2,
    monitorSeq: 3,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "0",
    catchingUp: "false",
    peerCount: "0",
    pid: 7,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "current");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "0");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Catching up"], "false");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Sync peer count"], "0");
  assert.doesNotMatch(JSON.stringify(readout(world.nodes.get("monitor-readout")!)), /synced|recovered/i);

  world.timers.get([...world.timers.keys()][0])!();
  world.take("node_status").resolve(snapshot({ state: "running", statusSeq: 2, monitorSeq: 3, pid: 7 }));
  world.take("monitor_refresh").resolve(snapshot({
    state: "running",
    statusSeq: 2,
    monitorSeq: 4,
    rpc: "unreachable",
    stale: true,
    lastSuccessAgeMs: 5000,
    height: "0",
    catchingUp: "false",
    detail: "health request failed: connection refused",
    pid: 7,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "stale last success 5s ago");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "0");
  assert.notEqual(readout(world.nodes.get("monitor-readout")!).Freshness, "current");

  world.timers.get([...world.timers.keys()][0])!();
  world.take("node_status").resolve(snapshot({ state: "running", statusSeq: 2, monitorSeq: 4, pid: 7 }));
  world.take("monitor_refresh").resolve(snapshot({
    state: "running",
    statusSeq: 2,
    monitorSeq: 5,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "12",
    catchingUp: "true",
    peerCount: null,
    pid: 7,
  }));
  await world.flush();
  const rows = readout(world.nodes.get("monitor-readout")!);
  assert.equal(rows.Freshness, "current");
  assert.equal(rows["Committed height"], "12");
  assert.equal(rows["Sync peer count"], "unknown");
  assert.notEqual(rows["Sync peer count"], "0");
  assert.equal(rows["Catching up"], "true");
  world.dispose();
});

test("polling does not accumulate timers and pause failures stay truthful", async () => {
  const world = harness();
  assert.equal(world.timers.size, 1);
  await boot(world);
  for (let count = 0; count < 5; count += 1) {
    world.document.hidden = count % 2 === 0;
    world.document.dispatch("visibilitychange");
  }
  assert.equal(world.timers.size, 1);
  const pauses = world.pending.filter((item) => item.command === "set_monitor_paused");
  assert.equal(pauses.length, 5);
  pauses[4].reject(new Error("pause failed: backend"));
  for (const pause of pauses.slice(0, 4)) pause.resolve(null);
  await world.flush();
  assert.equal(world.nodes.get("setup-error")!.hidden, false);
  assert.match(world.nodes.get("setup-error")!.textContent, /pause failed: backend/);

  world.document.hidden = false;
  world.document.dispatch("visibilitychange");
  const resume = world.pending.filter((item) => item.command === "set_monitor_paused").at(-1)!;
  world.document.hidden = true;
  world.document.dispatch("visibilitychange");
  const hiddenAgain = world.pending.filter((item) => item.command === "set_monitor_paused").at(-1)!;
  hiddenAgain.resolve(null);
  await world.flush();
  resume.reject(new Error("stale pause failure"));
  await world.flush();
  assert.doesNotMatch(world.nodes.get("setup-error")!.textContent, /stale pause failure/);

  world.document.hidden = false;
  world.timers.get([...world.timers.keys()][0])!();
  const status = world.take("node_status");
  const refresh = world.take("monitor_refresh");
  world.document.dispatch("visibilitychange");
  const visiblePause = world.pending.filter((item) => item.command === "set_monitor_paused").at(-1)!;
  visiblePause.resolve(null);
  await world.flush();
  assert.equal(world.pending.filter((item) => item.command === "monitor_refresh").length, 0);
  refresh.resolve(snapshot({ statusSeq: 1, monitorSeq: 2, rpc: "unreachable", detail: "still down", heightKind: "unavailable" }));
  status.resolve(snapshot({ statusSeq: 1, monitorSeq: 1 }));
  await world.flush();
  const followUp = world.pending.filter((item) => item.command === "monitor_refresh");
  assert.equal(followUp.length, 1);
  followUp[0].reject(new Error("refresh failed"));
  await world.flush();
  assert.match(world.nodes.get("setup-error")!.textContent, /refresh failed/);

  world.window.dispatch("beforeunload");
  assert.equal(world.timers.size, 0);
  world.dispose();
  const before = world.pending.length;
  world.document.dispatch("visibilitychange");
  assert.equal(world.pending.length, before);
  assert.equal(world.timers.size, 0);
});

test("keyboard start and stop follow the authoritative process and do not duplicate", async () => {
  const world = harness();
  await boot(world);
  const tuple = fillPaths(world);
  world.nodes.get("check-button")!.press("Enter");
  world.take("inspect_setup").resolve(checked(tuple));
  await world.flush();
  world.nodes.get("start-button")!.press(" ");
  world.nodes.get("start-button")!.press("Enter");
  const starts = world.pending.filter((item) => item.command === "node_start");
  assert.equal(starts.length, 1);
  assert.deepEqual(starts[0].args, { binary: tuple.binary, configPath: tuple.config, dataDir: tuple.data });
  const stale = snapshot({ statusSeq: 1, monitorSeq: 1 });
  world.timers.get([...world.timers.keys()][0])!();
  const oldStatus = world.take("node_status");
  starts[0].resolve(snapshot({
    state: "running",
    statusSeq: 3,
    monitorSeq: 2,
    rpc: "unknown",
    pid: 9,
    logs: ["started pid 9"],
  }));
  await world.flush();
  oldStatus.resolve(stale);
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "running");
  assert.equal(world.nodes.get("start-button")!.disabled, true);
  assert.equal(world.nodes.get("stop-button")!.disabled, false);
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "not observed");

  world.nodes.get("stop-button")!.press(" ");
  world.nodes.get("stop-button")!.press("Enter");
  assert.equal(world.pending.filter((item) => item.command === "node_stop").length, 1);
  const stop = world.take("node_stop");
  const lateStartError = starts[0];
  void lateStartError;
  stop.reject(new Error("owned pid 9 was not confirmed exited"));
  await world.flush();
  assert.match(world.nodes.get("setup-error")!.textContent, /not confirmed exited/);
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "running");
  assert.equal(world.nodes.get("stop-button")!.disabled, false);

  world.nodes.get("stop-button")!.press("Enter");
  const retry = world.take("node_stop");
  const olderStopError = stop;
  void olderStopError;
  retry.resolve(snapshot({
    state: "exited",
    statusSeq: 5,
    monitorSeq: 3,
    rpc: "unreachable",
    detail: "owned process exited with code 0; monitor is not live",
    heightKind: "unavailable",
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "exited");
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "unreachable");
  assert.equal(world.nodes.get("setup-error")!.hidden, true);
  world.dispose();
});

test("a pre-restart monitor cannot replace fresh post-restart data", async () => {
  const world = harness();
  await boot(world);
  const tuple = fillPaths(world);
  world.nodes.get("check-button")!.press("Enter");
  world.take("inspect_setup").resolve(checked(tuple));
  await world.flush();
  world.nodes.get("start-button")!.press("Enter");
  world.take("node_start").resolve(snapshot({
    state: "running",
    statusSeq: 3,
    monitorSeq: 2,
    rpc: "unknown",
    pid: 40,
    logs: ["started pid 40"],
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "not observed");

  world.timers.get([...world.timers.keys()][0])!();
  world.take("node_status").resolve(snapshot({ state: "running", statusSeq: 3, monitorSeq: 2, pid: 40 }));
  const firstRead = world.take("monitor_refresh");
  firstRead.resolve(snapshot({
    state: "running",
    statusSeq: 3,
    monitorSeq: 3,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "4",
    catchingUp: "false",
    pid: 40,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "4");
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "current");

  world.timers.get([...world.timers.keys()][0])!();
  const staleStatus = world.take("node_status");
  const staleRefresh = world.take("monitor_refresh");
  world.nodes.get("stop-button")!.press("Enter");
  world.take("node_stop").resolve(snapshot({
    state: "exited",
    statusSeq: 5,
    monitorSeq: 4,
    rpc: "unreachable",
    detail: "owned process exited with code 0; monitor is not live",
    heightKind: "unavailable",
    logs: ["started pid 40", "exited code 0"],
  }));
  await world.flush();
  staleStatus.resolve(snapshot({
    state: "running",
    statusSeq: 3,
    monitorSeq: 8,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "9",
    catchingUp: "false",
    pid: 40,
  }));
  staleRefresh.resolve(snapshot({
    state: "running",
    statusSeq: 3,
    monitorSeq: 9,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "9",
    catchingUp: "false",
    pid: 40,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "exited");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "unavailable");
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "unreachable");
  assert.notEqual(readout(world.nodes.get("monitor-readout")!)["Committed height"], "0");

  world.nodes.get("start-button")!.press("Enter");
  world.take("node_start").resolve(snapshot({
    state: "running",
    statusSeq: 6,
    monitorSeq: 5,
    rpc: "unknown",
    pid: 41,
    logs: ["started pid 41"],
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "running");
  assert.equal(readout(world.nodes.get("process-readout")!).PID, "41");
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "not observed");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "unknown");
  assert.notEqual(readout(world.nodes.get("monitor-readout")!)["Committed height"], "4");

  world.timers.get([...world.timers.keys()][0])!();
  const duringRestart = world.take("node_status");
  const freshRead = world.take("monitor_refresh");
  duringRestart.resolve(snapshot({
    state: "exited",
    statusSeq: 5,
    monitorSeq: 4,
    rpc: "unreachable",
    detail: "owned process exited with code 0; monitor is not live",
    heightKind: "unavailable",
    pid: null,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("process-readout")!).Process, "running");
  freshRead.resolve(snapshot({
    state: "running",
    statusSeq: 6,
    monitorSeq: 7,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "12",
    catchingUp: "true",
    peerCount: "0",
    pid: 41,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "12");
  assert.equal(readout(world.nodes.get("monitor-readout")!).Freshness, "current");
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Catching up"], "true");
  assert.doesNotMatch(JSON.stringify(readout(world.nodes.get("monitor-readout")!)), /synced|recovered/i);

  world.timers.get([...world.timers.keys()][0])!();
  world.take("node_status").resolve(snapshot({ state: "running", statusSeq: 6, monitorSeq: 7, pid: 41 }));
  world.take("monitor_refresh").resolve(snapshot({
    state: "running",
    statusSeq: 3,
    monitorSeq: 10,
    rpc: "reachable",
    lastSuccessAgeMs: 0,
    height: "4",
    catchingUp: "false",
    pid: 40,
  }));
  await world.flush();
  assert.equal(readout(world.nodes.get("monitor-readout")!)["Committed height"], "12");
  assert.equal(readout(world.nodes.get("process-readout")!).PID, "41");
  world.dispose();
});
