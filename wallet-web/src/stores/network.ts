import { create } from "zustand";
import { chainName, presetById, type PresetId } from "@/lib/networks";

const CHOICE_KEY = "kvnc.network.v1";

type NetworkState = {
  presetId: PresetId;
  rpcUrl: string;
  probedChainId: number | null;
  committedHeight: number | null;
  latencyMs: number | null;
  peerCount: number | null;
  online: boolean;
  probing: boolean;
  error: string | null;
  balanceAtoms: bigint | null;
  nonce: bigint | null;
  feeAtoms: bigint | null;
  indexerUrl: string;
  setIndexerUrl: (indexerUrl: string) => void;
  selectPreset: (presetId: PresetId) => void;
  setRpcUrl: (rpcUrl: string, presetId?: PresetId) => void;
  applyProbe: (probe: {
    chainId: number | null;
    height: number | null;
    latencyMs: number | null;
    peerCount: number | null;
    online: boolean;
    error: string | null;
  }) => void;
  setAccountState: (balanceAtoms: bigint | null, nonce: bigint | null, feeAtoms: bigint | null) => void;
  setProbing: (probing: boolean) => void;
};

function persistChoice(presetId: PresetId, rpcUrl: string) {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(CHOICE_KEY, JSON.stringify({ presetId, rpcUrl }));
}

export function savedNetworkChoice(): { presetId: PresetId; rpcUrl: string } | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const parsed = JSON.parse(localStorage.getItem(CHOICE_KEY) ?? "") as { presetId?: PresetId; rpcUrl?: string };
    if (!parsed?.presetId || !parsed.rpcUrl) return null;
    presetById(parsed.presetId);
    return { presetId: parsed.presetId, rpcUrl: parsed.rpcUrl };
  } catch {
    return null;
  }
}

export const useNetwork = create<NetworkState>((set) => ({
  presetId: "testnet",
  rpcUrl: presetById("testnet").rpcUrl,
  probedChainId: null,
  committedHeight: null,
  latencyMs: null,
  peerCount: null,
  online: false,
  probing: false,
  error: null,
  balanceAtoms: null,
  nonce: null,
  feeAtoms: null,
  indexerUrl: "",
  setIndexerUrl: (indexerUrl) => set({ indexerUrl }),
  selectPreset: (presetId) => {
    const rpcUrl = presetById(presetId).rpcUrl;
    persistChoice(presetId, rpcUrl);
    set({
      presetId,
      rpcUrl,
      probedChainId: null,
      committedHeight: null,
      online: false,
      error: null,
      balanceAtoms: null,
      nonce: null,
      feeAtoms: null,
    });
  },
  setRpcUrl: (rpcUrl, presetId) =>
    set((state) => {
      const nextId = presetId ?? state.presetId;
      persistChoice(nextId, rpcUrl);
      return {
        presetId: nextId,
        rpcUrl,
        probedChainId: null,
        committedHeight: null,
        online: false,
        error: null,
        balanceAtoms: null,
        nonce: null,
        feeAtoms: null,
      };
    }),
  applyProbe: (probe) =>
    set({
      probedChainId: probe.chainId,
      committedHeight: probe.height,
      latencyMs: probe.latencyMs,
      peerCount: probe.peerCount,
      online: probe.online,
      error: probe.error,
      probing: false,
    }),
  setAccountState: (balanceAtoms, nonce, feeAtoms) => set({ balanceAtoms, nonce, feeAtoms }),
  setProbing: (probing) => set({ probing }),
}));

export function signingLock(state: {
  presetId: PresetId;
  probedChainId: number | null;
  online: boolean;
}): { locked: boolean; reason: string; networkName: string } {
  const preset = presetById(state.presetId);
  const networkName = state.probedChainId == null ? preset.label : chainName(state.probedChainId);
  if (preset.frozen) {
    return { locked: true, reason: "Mainnet is frozen. This wallet can read it, not spend from it.", networkName };
  }
  if (!state.online || state.probedChainId == null) {
    return { locked: true, reason: "Signing is off until a node reports its chain id.", networkName };
  }
  if (preset.expectedChainId != null && state.probedChainId !== preset.expectedChainId) {
    return {
      locked: true,
      reason: `${preset.label} expects chain ${preset.expectedChainId}, but the node reported chain ${state.probedChainId}.`,
      networkName,
    };
  }
  return { locked: false, reason: "", networkName };
}