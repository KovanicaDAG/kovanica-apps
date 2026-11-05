import { invoke } from "@tauri-apps/api/core";

import {
  endpointLine,
  peerLines,
  statusRows,
  type ConnectionConfig,
  type NodeStatus,
  type StatusRow,
} from "./connection";
import { presetById, presetFor } from "./networks";
import { mountWallet } from "./wallet-view";
import "./styles.css";

let shownChainId: string | null = null;

/** One control from `index.html`; a missing id is a build error, not a null check. */
function control<T extends Element>(selector: string): T {
  const found = document.querySelector<T>(selector);
  if (!found) throw new Error(`Connection screen is missing ${selector}.`);
  return found;
}

const rpc = control<HTMLInputElement>("#rpc");
const endpointText = control<HTMLParagraphElement>("#endpoint");
const errorText = control<HTMLParagraphElement>("#error");
const statusNode = control<HTMLDListElement>("#status");
const peersNode = control<HTMLUListElement>("#peers");
const save = control<HTMLButtonElement>("#save");
const reset = control<HTMLButtonElement>("#reset");
const refresh = control<HTMLButtonElement>("#refresh");

const netTestnet = control<HTMLButtonElement>("#net-testnet");
const netMainnet = control<HTMLButtonElement>("#net-mainnet");
const netLocal = control<HTMLButtonElement>("#net-local");
const netCustom = control<HTMLButtonElement>("#net-custom");

function showError(message: string): void {
  errorText.hidden = false;
  errorText.textContent = message;
}

function clearError(): void {
  errorText.hidden = true;
  errorText.textContent = "";
}

function clearNode(): void {
  statusNode.replaceChildren();
  peersNode.replaceChildren();
}

function fillRows(rows: StatusRow[]): void {
  statusNode.replaceChildren();
  for (const row of rows) {
    const term = document.createElement("dt");
    term.textContent = row.label;
    const value = document.createElement("dd");
    value.textContent = row.value;
    statusNode.append(term, value);
  }
}

function fillPeers(lines: string[]): void {
  peersNode.replaceChildren();
  for (const line of lines) {
    const item = document.createElement("li");
    item.textContent = line;
    peersNode.append(item);
  }
}

function applyConfig(config: ConnectionConfig): void {
  rpc.value = config.rpcUrl;
  endpointText.textContent = endpointLine(config.endpointLabel);
  updateNetworkButtons(config.rpcUrl);
}

function updateNetworkButtons(url: string): void {
  const active = presetFor(url)?.id ?? null;
  netTestnet.classList.toggle("active", active === "testnet");
  netMainnet.classList.toggle("active", active === "mainnet");
  netLocal.classList.toggle("active", active === "local");
  netCustom.classList.toggle("active", active === null);
}

/** The switch labels come from `networks.ts`, the page only holds the buttons. */
function labelNetworkButtons(): void {
  netTestnet.textContent = presetById("testnet")?.label ?? "Testnet";
  netMainnet.textContent = presetById("mainnet")?.label ?? "Mainnet";
  netLocal.textContent = presetById("local")?.label ?? "Local";
}

function messageFrom(error: unknown): string {
  if (typeof error === "string" && error.trim() !== "") return error;
  if (error instanceof Error && error.message.trim() !== "") return error.message;
  return "The node is not reachable.";
}

async function loadConfig(): Promise<void> {
  applyConfig(await invoke<ConnectionConfig>("connection_config"));
}

async function refreshStatus(): Promise<void> {
  refresh.disabled = true;
  clearError();
  try {
    const status = await invoke<NodeStatus>("node_status");
    shownChainId = status.chainId;
    fillRows(statusRows(status));
    fillPeers(peerLines(status.peers));
  } catch (error) {
    shownChainId = null;
    clearNode();
    showError(messageFrom(error));
  } finally {
    refresh.disabled = false;
  }
}

save.addEventListener("click", () => {
  void (async () => {
    clearError();
    try {
      applyConfig(await invoke<ConnectionConfig>("save_connection", { rpcUrl: rpc.value }));
      await refreshStatus();
    } catch (error) {
      showError(messageFrom(error));
    }
  })();
});

reset.addEventListener("click", () => {
  void (async () => {
    clearError();
    try {
      applyConfig(await invoke<ConnectionConfig>("reset_connection"));
      await refreshStatus();
    } catch (error) {
      showError(messageFrom(error));
    }
  })();
});

refresh.addEventListener("click", () => {
  void refreshStatus();
});

/** Save a built-in endpoint and read the node again. */
function selectPreset(url: string): void {
  rpc.value = url;
  void (async () => {
    clearError();
    try {
      applyConfig(await invoke<ConnectionConfig>("save_connection", { rpcUrl: url }));
      await refreshStatus();
    } catch (error) {
      showError(messageFrom(error));
    }
  })();
}

netTestnet.addEventListener("click", () => {
  const preset = presetById("testnet");
  if (preset) selectPreset(preset.rpcUrl);
});
netMainnet.addEventListener("click", () => {
  const preset = presetById("mainnet");
  if (preset) selectPreset(preset.rpcUrl);
});
netLocal.addEventListener("click", () => {
  const preset = presetById("local");
  if (preset) selectPreset(preset.rpcUrl);
});

netCustom.addEventListener("click", () => {
  rpc.focus();
  rpc.select();
  updateNetworkButtons(rpc.value);
});

labelNetworkButtons();
mountWallet(() => shownChainId);

void (async () => {
  try {
    await loadConfig();
    await refreshStatus();
  } catch (error) {
    showError(messageFrom(error));
  }
})();
