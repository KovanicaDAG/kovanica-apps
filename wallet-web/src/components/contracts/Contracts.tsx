import { useState } from "react";
import { checkAddress } from "@/core/address";
import { parseAtoms, parseKvnc } from "@/core/format";
import type { TxPreview } from "@/core/session";
import { presetById } from "@/lib/networks";
import { buildCallPreview } from "@/services/wallet";
import { signingLock, useNetwork } from "@/stores/network";

const ACTIONS = [
  { id: "vault", label: "Vault", method: "claim", hint: "Claim vested KVNC" },
  { id: "htlc", label: "HTLC", method: "claim", hint: "Create, fund, or claim a hash lock" },
  { id: "multisig", label: "Multisig", method: "propose", hint: "Propose or add a signature" },
  { id: "token", label: "Token", method: "transfer", hint: "Transfer a fungible token" },
] as const;

export function Contracts({ onReview }: { onReview: (preview: TxPreview) => void }) {
  const net = useNetwork();
  const lock = signingLock(net);
  const policyChain = net.probedChainId ?? presetById(net.presetId).expectedChainId;
  const [action, setAction] = useState<(typeof ACTIONS)[number]["id"]>("vault");
  const [method, setMethod] = useState("claim");
  const [contract, setContract] = useState("");
  const [value, setValue] = useState("0");
  const [gas, setGas] = useState("100000");
  const [args, setArgs] = useState("");
  const [error, setError] = useState<string | null>(null);
  const selected = ACTIONS.find((item) => item.id === action)!;
  const address = checkAddress(contract, policyChain);

  function review() {
    setError(null);
    if (lock.locked || net.probedChainId == null || net.nonce == null || net.feeAtoms == null) {
      return setError(lock.locked ? lock.reason : "Balance, fee, and nonce are not available yet.");
    }
    if (!address.ok) return setError(address.message);
    const valueAtoms = parseKvnc(value);
    const gasLimit = parseAtoms(gas);
    if (valueAtoms == null || gasLimit == null) return setError("Value and gas must be integers in their fields.");
    if (args.length % 2 !== 0 || (args.length > 0 && !/^[0-9a-f]+$/i.test(args))) return setError("Args must be even-length hex.");
    const bytes = new Uint8Array(args.length / 2);
    for (let i = 0; i < bytes.length; i += 1) bytes[i] = Number.parseInt(args.slice(i * 2, i * 2 + 2), 16);
    try {
      onReview(buildCallPreview({
        contract,
        method,
        args: bytes,
        valueAtoms,
        gasLimit,
        feeAtoms: net.feeAtoms,
        nonce: net.nonce,
        chainId: net.probedChainId,
        networkName: lock.networkName,
      }));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not build the call");
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(e) => { e.preventDefault(); review(); }}>
      <div className="grid grid-cols-2 gap-2">
        {ACTIONS.map((item) => (
          <button
            key={item.id}
            type="button"
            className={`min-h-11 rounded-2xl border px-3 text-sm ${action === item.id ? "border-copper text-paper" : "border-line text-muted"}`}
            onClick={() => { setAction(item.id); setMethod(item.method); }}
          >
            {item.label}
          </button>
        ))}
      </div>
      <p className="text-sm text-muted">{selected.hint}. The same confirm screen signs the call.</p>
      <label className="grid gap-1 text-sm text-muted">
        Contract
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 font-mono text-sm text-paper" value={contract} onChange={(e) => setContract(e.target.value)} autoCapitalize="none" spellCheck={false} />
      </label>
      {contract.trim() ? <p className={address.ok ? "text-sm text-sage" : "text-sm text-clay"}>{address.message}</p> : null}
      <label className="grid gap-1 text-sm text-muted">
        Method
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" value={method} onChange={(e) => setMethod(e.target.value)} />
      </label>
      <label className="grid gap-1 text-sm text-muted">
        Value (KVNC)
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 tabular-nums text-paper" value={value} onChange={(e) => setValue(e.target.value)} />
      </label>
      <label className="grid gap-1 text-sm text-muted">
        Gas limit
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 tabular-nums text-paper" value={gas} onChange={(e) => setGas(e.target.value)} />
      </label>
      <label className="grid gap-1 text-sm text-muted">
        Args hex
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 font-mono text-sm text-paper" value={args} onChange={(e) => setArgs(e.target.value.trim())} />
      </label>
      {error ? <p className="text-sm text-clay">{error}</p> : null}
      <button type="submit" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={lock.locked}>
        Review call
      </button>
    </form>
  );
}
