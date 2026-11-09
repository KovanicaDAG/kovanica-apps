import { create } from "zustand";
import { chainName, presetById, type PresetId } from "@/lib/networks";

type NetworkState = {
  presetId: PresetId;
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

export const useNetwork = create<NetworkState>((set) => ({
  presetId: "testnet",
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
  selectPreset: (presetId) =>
    set({
      presetId,
      probedChainId: null,
      committedHeight: null,
      online: false,
      error: null,
      balanceAtoms: null,
      nonce: null,
      feeAtoms: null,
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
