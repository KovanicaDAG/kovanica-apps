import { useEffect, useState } from "react";
import { checkAddress } from "@/core/address";
import { formatAtoms, formatKvnc, parseAtoms, parseKvnc } from "@/core/format";
import type { TxPreview } from "@/core/session";
import { ContactSelect } from "@/components/contacts/ContactSelect";
import { buildTransferPreview } from "@/services/wallet";
import { signingLock, useNetwork } from "@/stores/network";
import { presetById } from "@/lib/networks";

export function SendForm({ onReview, prefillTo = "" }: { onReview: (preview: TxPreview) => void; prefillTo?: string }) {
  const net = useNetwork();
  const lock = signingLock(net);
  const policyChain = net.probedChainId ?? presetById(net.presetId).expectedChainId;
  const [to, setTo] = useState(prefillTo);
  const [amount, setAmount] = useState("");
  const [feeText, setFeeText] = useState("");
  const [nonceText, setNonceText] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const address = checkAddress(to, policyChain);
  const fee = feeText.trim() ? parseAtoms(feeText) : net.feeAtoms;
  const amountAtoms = parseKvnc(amount);
  const nonce = nonceText.trim() ? parseAtoms(nonceText) : net.nonce;

  useEffect(() => {
    if (prefillTo) setTo(prefillTo);
  }, [prefillTo]);

  function max() {
    if (net.balanceAtoms == null || fee == null) return;
    const next = net.balanceAtoms > fee ? net.balanceAtoms - fee : 0n;
    setAmount(formatKvnc(next));
  }

  function review() {
    setError(null);
    if (lock.locked || net.probedChainId == null) return setError(lock.reason);
    if (!address.ok) return setError(address.message);
    if (amountAtoms == null || amountAtoms <= 0n) return setError("Enter an amount in KVNC, up to 9 decimal places.");
    if (fee == null) return setError("Fee is unknown. Enter it under Advanced.");
    if (nonce == null) return setError("Nonce is unknown. Enter it under Advanced.");
    if (net.balanceAtoms != null && amountAtoms + fee > net.balanceAtoms) return setError("Amount plus fee is above the balance.");
    try {
      onReview(buildTransferPreview({
        recipient: to,
        amountAtoms,
        feeAtoms: fee,
        nonce,
        chainId: net.probedChainId,
        networkName: lock.networkName,
      }));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not build the preview");
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(e) => { e.preventDefault(); review(); }}>
      <label className="grid gap-1 text-sm text-muted">
        Recipient
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 font-mono text-sm text-paper" value={to} onChange={(e) => setTo(e.target.value)} autoCapitalize="none" autoCorrect="off" spellCheck={false} placeholder="kvnc…dag" />
      </label>
      <ContactSelect onPick={setTo} />
      {to.trim() ? <p className={address.ok ? "text-sm text-sage" : "text-sm text-clay"}>{address.message}</p> : null}
      <label className="grid gap-1 text-sm text-muted">
        Amount (KVNC)
        <div className="flex gap-2">
          <input className="min-h-11 flex-1 rounded-2xl border border-line bg-ink px-4 tabular-nums text-paper" inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} placeholder="0.000000000" />
          <button type="button" className="min-h-11 rounded-full border border-line px-4 text-paper" onClick={max}>Max</button>
        </div>
      </label>
      <p className="text-sm text-muted">
        Fee {fee == null ? "unavailable" : `${formatKvnc(fee)} KVNC (${formatAtoms(fee)} atoms)`}
        {net.balanceAtoms != null ? ` · balance ${formatKvnc(net.balanceAtoms)} KVNC` : ""}
      </p>
      <button type="button" className="text-left text-sm text-copper" onClick={() => setAdvanced((v) => !v)}>
        {advanced ? "Hide advanced" : "Advanced nonce and fee"}
      </button>
      {advanced ? (
        <div className="grid gap-3">
          <label className="grid gap-1 text-sm text-muted">
            Nonce
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 tabular-nums text-paper" value={nonceText} placeholder={net.nonce?.toString() ?? ""} onChange={(e) => setNonceText(e.target.value)} />
          </label>
          <label className="grid gap-1 text-sm text-muted">
            Fee atoms
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 tabular-nums text-paper" value={feeText} placeholder={net.feeAtoms?.toString() ?? ""} onChange={(e) => setFeeText(e.target.value)} />
          </label>
          <p className="text-xs text-muted">A repeated nonce is not a cancel button unless the node replaces it.</p>
        </div>
      ) : null}
      {lock.locked ? <p className="text-sm text-clay">{lock.reason}</p> : null}
      {error ? <p className="text-sm text-clay">{error}</p> : null}
      <button type="submit" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={lock.locked}>
        Review transaction
      </button>
    </form>
  );
}
