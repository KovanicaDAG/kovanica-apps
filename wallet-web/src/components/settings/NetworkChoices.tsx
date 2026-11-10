import { useState } from "react";
import { FALLBACK_RPC, PRESETS, normalizeRpc } from "@/lib/networks";
import { useNetwork } from "@/stores/network";
import { toast } from "sonner";

export function NetworkChoices() {
  const net = useNetwork();
  const [host, setHost] = useState("");

  function useHost(raw: string, presetId?: "testnet" | "hybrid" | "local" | "mainnet") {
    try {
      useNetwork.getState().setRpcUrl(normalizeRpc(raw), presetId);
      setHost("");
    } catch (error) {
      toast(error instanceof Error ? error.message : "That node address was rejected");
    }
  }

  return (
    <div className="grid gap-2">
      {PRESETS.map((preset) => (
        <button
          key={preset.id}
          type="button"
          className={`min-h-11 rounded-2xl border px-3 text-left text-xs ${net.presetId === preset.id ? "border-copper text-copper" : "border-line text-muted"}`}
          onClick={() => useNetwork.getState().selectPreset(preset.id)}
        >
          <span className="block">{preset.recommended ? "Recommended" : preset.frozen ? "Frozen" : "Network"}</span>
          <span className="block text-sm text-paper">{preset.label}</span>
        </button>
      ))}
      <p className="break-all text-xs text-muted">{net.rpcUrl}</p>
      {!net.online && !net.probing ? (
        <div className="grid gap-2 rounded-2xl border border-line p-3">
          <p className="text-sm text-paper">This node did not respond.</p>
          <button type="button" className="min-h-11 rounded-full border border-copper px-3 text-sm text-copper" onClick={() => useNetwork.getState().setRpcUrl(FALLBACK_RPC, "testnet")}>
            Use rpc-testnet.kovanica.online
          </button>
          <form className="grid gap-2" onSubmit={(event) => { event.preventDefault(); useHost(host); }}>
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-3 text-sm text-paper" value={host} onChange={(event) => setHost(event.target.value)} placeholder="192.168.1.10:8545" />
            <button type="submit" className="min-h-11 rounded-full border border-line text-sm text-paper">Use this node</button>
          </form>
        </div>
      ) : null}
    </div>
  );
}
