import { useEffect, useState } from "react";
import { checkAddress } from "@/core/address";
import { buildContractCall } from "@/core/contracts";
import { parseAtoms, parseKvnc } from "@/core/format";
import type { TxPreview } from "@/core/session";
import { ContactSelect } from "@/components/contacts/ContactSelect";
import { presetById } from "@/lib/networks";
import { buildCallPreview } from "@/services/wallet";
import { signingLock, useNetwork } from "@/stores/network";
import { hexToBytes } from "@/core/bytes";

const CONTRACTS = [
  { id: "vault", label: "Vault", methods: ["claim", "withdraw"] },
  { id: "htlc", label: "HTLC", methods: ["lock", "claim", "refund"] },
  { id: "multisig", label: "Multisig", methods: ["propose", "approve", "execute"] },
  { id: "token", label: "Token", methods: ["transfer", "mint", "burn"] },
] as const;

type ContractId = (typeof CONTRACTS)[number]["id"];

export function Contracts({ onReview, prefill }: { onReview: (preview: TxPreview) => void; prefill?: { address: string; kind: ContractId } | null }) {
  const net = useNetwork();
  const lock = signingLock(net);
  const policyChain = net.probedChainId ?? presetById(net.presetId).expectedChainId;
  const [contractId, setContractId] = useState<ContractId>("vault");
  const [method, setMethod] = useState("claim");
  const [contract, setContract] = useState("");
  const [recipient, setRecipient] = useState("");
  const [account, setAccount] = useState("");
  const [kvnc, setKvnc] = useState("");
  const [units, setUnits] = useState("");
  const [hashlock, setHashlock] = useState("");
  const [preimage, setPreimage] = useState("");
  const [height, setHeight] = useState("");
  const [lockId, setLockId] = useState("");
  const [proposalId, setProposalId] = useState("");
  const [callData, setCallData] = useState("");
  const [value, setValue] = useState("0");
  const [gas, setGas] = useState("100000");
  const [advanced, setAdvanced] = useState(false);
  const [raw, setRaw] = useState("");
  const [error, setError] = useState<string | null>(null);
  const selected = CONTRACTS.find((item) => item.id === contractId)!;
  const address = checkAddress(contract, policyChain);

  useEffect(() => {
    if (!prefill) return;
    setContract(prefill.address);
    setContractId(prefill.kind);
    const next = CONTRACTS.find((item) => item.id === prefill.kind);
    if (next) setMethod(next.methods[0]);
  }, [prefill]);

  function needs(name: string) {
    return (
      ((contractId === "vault" && method === "withdraw") ||
        (contractId === "htlc" && method === "lock") ||
        (contractId === "multisig" && method === "propose") ||
        (contractId === "token" && method === "transfer")) &&
      name === "recipient"
    ) || ((contractId === "token" && (method === "mint" || method === "burn")) && name === "account")
      || (((contractId === "vault" && method === "withdraw") || (contractId === "multisig" && method === "propose")) && name === "kvnc")
      || (contractId === "token" && name === "units")
      || (contractId === "htlc" && method === "lock" && (name === "hashlock" || name === "height"))
      || (contractId === "htlc" && method === "claim" && (name === "lock" || name === "preimage"))
      || (contractId === "htlc" && method === "refund" && name === "lock")
      || (contractId === "multisig" && method === "propose" && name === "calldata")
      || (contractId === "multisig" && (method === "approve" || method === "execute") && name === "proposal");
  }

  function review() {
    setError(null);
    if (lock.locked || net.probedChainId == null || net.nonce == null || net.feeAtoms == null) {
      return setError(lock.locked ? lock.reason : "Balance, fee, and nonce are not available yet.");
    }
    if (!address.ok) return setError(address.message);
    const valueAtoms = parseKvnc(value);
    const gasLimit = parseAtoms(gas);
    if (valueAtoms == null || gasLimit == null) return setError("Value and gas must be valid integers.");
    try {
      const draft = advanced
        ? { method, args: raw ? hexToBytes(raw.replace(/^0x/, "")) : new Uint8Array(), details: [{ label: "Args", value: "Advanced raw hex" }] }
        : buildContractCall({
            contract: contractId,
            method,
            chainId: policyChain,
            recipient,
            account,
            amountAtoms: kvnc.trim() ? parseKvnc(kvnc) : null,
            tokenUnits: units.trim() ? parseAtoms(units) : null,
            hashlock,
            timelockHeight: height.trim() ? parseAtoms(height) : null,
            lockId: lockId.trim() ? parseAtoms(lockId) : null,
            secretPreimage: preimage,
            proposalId: proposalId.trim() ? parseAtoms(proposalId) : null,
            callDataHex: callData,
          });
      if (!advanced && (method === "withdraw" || method === "propose") && parseKvnc(kvnc) == null) {
        return setError("Enter the amount in KVNC.");
      }
      onReview(buildCallPreview({
        contract,
        method: draft.method,
        args: draft.args,
        valueAtoms,
        gasLimit,
        feeAtoms: net.feeAtoms,
        nonce: net.nonce,
        chainId: net.probedChainId,
        networkName: lock.networkName,
        details: draft.details,
      }));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not build the call");
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(event) => { event.preventDefault(); review(); }}>
      <div className="grid grid-cols-2 gap-2">
        {CONTRACTS.map((item) => (
          <button key={item.id} type="button" className={`min-h-11 rounded-2xl border px-3 text-sm ${contractId === item.id ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => { setContractId(item.id); setMethod(item.methods[0]); }}>
            {item.label}
          </button>
        ))}
      </div>
      <div className="flex flex-wrap gap-2">
        {selected.methods.map((item) => (
          <button key={item} type="button" className={`min-h-11 rounded-full border px-4 text-sm ${method === item ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => setMethod(item)}>
            {item}
          </button>
        ))}
      </div>
      <label className="grid gap-1 text-sm text-muted">
        Contract address
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 font-mono text-sm text-paper" value={contract} onChange={(e) => setContract(e.target.value)} autoCapitalize="none" spellCheck={false} />
      </label>
      <ContactSelect onPick={setContract} />
      {contract.trim() ? <p className={address.ok ? "text-sm text-sage" : "text-sm text-clay"}>{address.message}</p> : null}
      {needs("recipient") ? (
        <>
          <ContactSelect onPick={setRecipient} />
          <Field label="Recipient" value={recipient} onChange={setRecipient} mono />
        </>
      ) : null}
      {needs("account") ? (
        <>
          <ContactSelect onPick={setAccount} />
          <Field label="Account" value={account} onChange={setAccount} mono />
        </>
      ) : null}
      {needs("kvnc") ? <Field label="Amount (KVNC)" value={kvnc} onChange={setKvnc} /> : null}
      {needs("units") ? <Field label="Token units" value={units} onChange={setUnits} /> : null}
      {needs("hashlock") ? <Field label="Hashlock hex" value={hashlock} onChange={setHashlock} mono /> : null}
      {needs("preimage") ? <Field label="Secret preimage hex" value={preimage} onChange={setPreimage} mono /> : null}
      {needs("height") ? <Field label="Timelock height" value={height} onChange={setHeight} /> : null}
      {needs("lock") ? <Field label="Lock ID" value={lockId} onChange={setLockId} /> : null}
      {needs("proposal") ? <Field label="Proposal ID" value={proposalId} onChange={setProposalId} /> : null}
      {needs("calldata") ? <Field label="Call data hex, optional" value={callData} onChange={setCallData} mono /> : null}
      <Field label="Value (KVNC)" value={value} onChange={setValue} />
      <Field label="Gas limit" value={gas} onChange={setGas} />
      <button type="button" className="text-left text-sm text-copper" onClick={() => setAdvanced((open) => !open)}>
        {advanced ? "Use the typed form" : "Advanced raw hex"}
      </button>
      {advanced ? <Field label="Args hex" value={raw} onChange={setRaw} mono /> : null}
      {error ? <p className="text-sm text-clay">{error}</p> : null}
      <button type="submit" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={lock.locked}>
        Review call
      </button>
    </form>
  );
}

function Field({ label, value, onChange, mono }: { label: string; value: string; onChange: (value: string) => void; mono?: boolean }) {
  return (
    <label className="grid gap-1 text-sm text-muted">
      {label}
      <input className={`min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper ${mono ? "font-mono text-sm" : "tabular-nums"}`} value={value} onChange={(event) => onChange(event.target.value)} autoCapitalize="none" spellCheck={false} />
    </label>
  );
}
