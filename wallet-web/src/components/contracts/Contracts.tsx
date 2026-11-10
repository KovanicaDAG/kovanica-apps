import { useEffect, useState } from "react";
import { checkAddress, decodeRecipient } from "@/core/address";
import { equalBytes } from "@/core/bytes";
import {
  buildContractCall,
  decodeTokenState,
  decodeVault,
  prefixedSlot,
  tokenStateSlot,
  type CallDraft,
  type ContractName,
} from "@/core/contracts";
import { parseAtoms } from "@/core/format";
import type { TxPreview } from "@/core/session";
import { ContactSelect } from "@/components/contacts/ContactSelect";
import { presetById } from "@/lib/networks";
import { buildCallPreview } from "@/services/wallet";
import { readStorageAt } from "@/services/rpc";
import { signingLock, useNetwork } from "@/stores/network";
import { useActivity } from "@/stores/activity";

const ACTIONS = {
  token: [
    { id: "balance_of", label: "balance_of", sign: false },
    { id: "token_transfer", label: "transfer", sign: true },
    { id: "token_mint", label: "mint", sign: true },
  ],
  htlc: [
    { id: "htlc_create", label: "create", sign: true },
    { id: "htlc_claim", label: "claim", sign: true },
    { id: "htlc_refund", label: "refund", sign: true },
  ],
  vault: [
    { id: "vault_create", label: "deposit", sign: true },
    { id: "vault_claim", label: "withdraw", sign: true },
    { id: "status", label: "status", sign: false },
  ],
  multisig: [
    { id: "multisig_propose", label: "propose", sign: true },
    { id: "multisig_confirm", label: "confirm", sign: true },
    { id: "multisig_execute", label: "execute", sign: true },
  ],
} as const;

type ActionId = (typeof ACTIONS)[ContractName][number]["id"];

export function Contracts({
  onReview,
  prefill,
  ownerAddress = "",
}: {
  onReview: (preview: TxPreview) => void;
  prefill?: { address: string; kind: ContractName } | null;
  ownerAddress?: string;
}) {
  const net = useNetwork();
  const lock = signingLock(net);
  const policyChain = net.probedChainId ?? presetById(net.presetId).expectedChainId;
  const [contractId, setContractId] = useState<ContractName>("token");
  const [action, setAction] = useState<ActionId>("balance_of");
  const [contract, setContract] = useState("");
  const [owner, setOwner] = useState(ownerAddress);
  const [recipient, setRecipient] = useState("");
  const [amount, setAmount] = useState("");
  const [hash, setHash] = useState("");
  const [timelock, setTimelock] = useState("");
  const [secret, setSecret] = useState("");
  const [recordId, setRecordId] = useState("");
  const [proposal, setProposal] = useState("");
  const [callData, setCallData] = useState("");
  const [value, setValue] = useState("0");
  const [gas, setGas] = useState("100000");
  const [error, setError] = useState<string | null>(null);
  const [readout, setReadout] = useState<string | null>(null);
  const [reading, setReading] = useState(false);
  const selected = ACTIONS[contractId];
  const current = selected.find((item) => item.id === action) ?? selected[0];
  const address = checkAddress(contract, policyChain);

  useEffect(() => {
    if (ownerAddress) setOwner((currentOwner) => currentOwner || ownerAddress);
  }, [ownerAddress]);

  useEffect(() => {
    if (!prefill) return;
    setContract(prefill.address);
    setContractId(prefill.kind);
    setAction(ACTIONS[prefill.kind][0].id);
  }, [prefill]);

  function pickContract(next: ContractName) {
    setContractId(next);
    setAction(ACTIONS[next][0].id);
    setError(null);
    setReadout(null);
  }

  function needs(name: string): boolean {
    const id = current.id;
    if (name === "owner") return id === "balance_of" || id === "token_mint";
    if (name === "recipient") return id === "token_transfer" || id === "htlc_create" || id === "vault_create" || id === "multisig_propose";
    if (name === "amount") return id === "token_transfer" || id === "token_mint" || id === "htlc_create" || id === "vault_create" || id === "multisig_propose";
    if (name === "hash") return id === "htlc_create";
    if (name === "timelock") return id === "htlc_create" || id === "vault_create";
    if (name === "secret") return id === "htlc_claim";
    if (name === "record") return id === "htlc_claim" || id === "htlc_refund" || id === "vault_claim" || id === "status" || id === "multisig_propose" || id === "multisig_confirm" || id === "multisig_execute";
    if (name === "proposal") return id === "multisig_confirm" || id === "multisig_execute";
    if (name === "calldata") return id === "multisig_propose";
    return false;
  }

  function whole(text: string, label: string): bigint {
    const parsed = parseAtoms(text);
    if (parsed == null) throw new Error(`${label} must be a whole number of atoms, not a decimal.`);
    return parsed;
  }

  function hex32(text: string, label: string): string {
    const clean = text.trim().toLowerCase().replace(/^0x/, "");
    if (!/^[0-9a-f]{64}$/.test(clean)) throw new Error(`${label} must be 64 hex characters.`);
    return clean;
  }

  function draft(): CallDraft {
    const amountAtoms = needs("amount") ? whole(amount, "Amount") : null;
    if (amountAtoms != null && amountAtoms <= 0n) throw new Error("Amount must be greater than zero.");
    return buildContractCall({
      contract: contractId,
      method: current.id,
      chainId: policyChain,
      claimer: recipient,
      beneficiary: recipient,
      recipient,
      account: owner,
      hash,
      secret,
      id: recordId,
      timelockMs: needs("timelock") && current.id === "htlc_create" ? whole(timelock, "Timelock") : null,
      lockUntilMs: current.id === "vault_create" ? whole(timelock, "Lock until") : null,
      amountAtoms,
      tokenUnits: current.id === "token_transfer" || current.id === "token_mint" ? amountAtoms : null,
      txId: needs("proposal") ? whole(proposal, "Proposal id") : null,
      callDataHex: callData,
    });
  }

  function review() {
    setError(null);
    setReadout(null);
    if (!current.sign) return;
    if (lock.locked || net.probedChainId == null || net.nonce == null || net.feeAtoms == null) {
      setError(lock.locked ? lock.reason : "Balance, fee, and nonce are not available yet.");
      return;
    }
    if (!address.ok) {
      setError(address.message);
      return;
    }
    const valueAtoms = parseAtoms(value);
    const gasLimit = parseAtoms(gas);
    if (valueAtoms == null || gasLimit == null) {
      setError("Attached value and gas limit must be whole numbers of atoms.");
      return;
    }
    try {
      if (needs("hash")) hex32(hash, "Hash");
      if (needs("secret") && !secret.trim()) throw new Error("Enter the secret as hex or text.");
      const built = draft();
      onReview(buildCallPreview({
        contract,
        method: built.method,
        args: built.args,
        valueAtoms,
        gasLimit,
        feeAtoms: net.feeAtoms,
        nonce: net.nonce,
        chainId: net.probedChainId,
        networkName: lock.networkName,
        details: [{ label: "Action", value: current.label }, ...built.details],
      }));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not build the call");
    }
  }

  async function readState() {
    setError(null);
    setReadout(null);
    if (!address.ok) {
      setError(address.message);
      return;
    }
    if (!net.online) {
      setError("The node is not responding, so this contract cannot be read.");
      return;
    }
    setReading(true);
    try {
      if (current.id === "balance_of") {
        const ownerCheck = checkAddress(owner, policyChain);
        if (!ownerCheck.ok) throw new Error(ownerCheck.message);
        const key = decodeRecipient(owner, policyChain);
        const raw = await readStorageAt(net.rpcUrl, contract, tokenStateSlot());
        const state = raw ? decodeTokenState(raw) : null;
        if (!state) throw new Error("This contract has no token state yet.");
        const row = state.balances.find((item) => equalBytes(item.account, key));
        setReadout(`${state.name} (${state.symbol}) balance_of ${owner.trim().toLowerCase()}: ${(row?.amount ?? 0n).toString()} base units. Supply ${state.totalSupply.toString()}. Decimals ${state.decimals}.`);
        return;
      }
      const id = hexToId(recordId);
      const prefix = current.id === "status" ? "kvnc/v1/vault/" : "";
      if (!prefix) throw new Error("That read is not available.");
      const raw = await readStorageAt(net.rpcUrl, contract, prefixedSlot(prefix, id));
      const vault = raw ? decodeVault(raw) : null;
      if (!vault) throw new Error("No vault record at that id.");
      setReadout(`Vault status: total ${vault.total.toString()} atoms, claimed ${vault.claimed.toString()}, schedule ${vault.schedule}, cancelled ${vault.cancelled ? "yes" : "no"}.`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not read the contract");
    } finally {
      setReading(false);
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(event) => { event.preventDefault(); if (current.sign) review(); else void readState(); }}>
      <div>
        <h2 className="text-2xl font-semibold">Contracts</h2>
        <p className="mt-2 text-sm text-muted">Canonical WASM entry points. A call is not signed until you confirm the method, arguments, fee, and gas. Reads never ask for the seed.</p>
      </div>
      <div className="grid grid-cols-2 gap-2">
        {(Object.keys(ACTIONS) as ContractName[]).map((item) => (
          <button key={item} type="button" className={`min-h-11 rounded-2xl border px-3 text-sm capitalize ${contractId === item ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => pickContract(item)}>
            {item}
          </button>
        ))}
      </div>
      <div className="flex flex-wrap gap-2">
        {selected.map((item) => (
          <button key={item.id} type="button" className={`min-h-11 rounded-full border px-4 text-sm ${action === item.id ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => { setAction(item.id); setError(null); setReadout(null); }}>
            {item.label}
          </button>
        ))}
      </div>
      <p className="text-sm text-muted">On-chain method: <span className="font-mono text-paper">{current.sign ? current.id : "kovanica_getStorageAt"}</span></p>
      <Field label="Contract address" value={contract} onChange={setContract} mono />
      <ContactSelect onPick={setContract} />
      {contract.trim() ? <p className={address.ok ? "text-sm text-sage" : "text-sm text-clay"}>{address.message}</p> : null}
      {needs("owner") ? (
        <>
          <Field label="Owner" value={owner} onChange={setOwner} mono />
          <ContactSelect onPick={setOwner} />
        </>
      ) : null}
      {needs("recipient") ? (
        <>
          <Field label={current.id === "htlc_create" ? "Recipient (claimer)" : current.id === "vault_create" ? "Beneficiary" : "Recipient"} value={recipient} onChange={setRecipient} mono />
          <ContactSelect onPick={setRecipient} />
        </>
      ) : null}
      {needs("amount") ? <Field label="Amount (atoms)" value={amount} onChange={setAmount} /> : null}
      {needs("hash") ? <Field label="Hash (32-byte hex)" value={hash} onChange={setHash} mono /> : null}
      {needs("timelock") ? <Field label={current.id === "vault_create" ? "Lock until (unix ms)" : "Timelock (unix ms)"} value={timelock} onChange={setTimelock} /> : null}
      {needs("secret") ? <Field label="Secret (hex or text)" value={secret} onChange={setSecret} mono /> : null}
      {needs("record") ? <Field label={contractId === "multisig" ? "Multisig id (32-byte hex)" : contractId === "htlc" ? "Swap id (32-byte hex)" : "Vault id (32-byte hex)"} value={recordId} onChange={setRecordId} mono /> : null}
      {needs("proposal") ? <Field label="Proposal id" value={proposal} onChange={setProposal} /> : null}
      {needs("calldata") ? <Field label="Call data hex, optional" value={callData} onChange={setCallData} mono /> : null}
      {current.id === "vault_claim" ? <p className="text-sm text-muted">Withdraw claims the vested balance. The contract does not take an amount; the beneficiary receives whatever is unlocked.</p> : null}
      {current.id === "htlc_create" || current.id === "vault_create" ? <p className="text-sm text-muted">The contract pulls Amount from your balance. Leave attached value at 0 unless this call also pays Call.value.</p> : null}
      {current.sign ? (
        <>
          <Field label="Attached value (atoms)" value={value} onChange={setValue} />
          <Field label="Gas limit" value={gas} onChange={setGas} />
          <p className="text-sm text-muted">Fee {net.feeAtoms == null ? "unavailable" : `${net.feeAtoms.toString()} atoms`} · nonce {net.nonce == null ? "unavailable" : net.nonce.toString()}</p>
        </>
      ) : null}
      {lock.locked && current.sign ? <p className="text-sm text-clay">{lock.reason}</p> : null}
      {error ? <p className="text-sm text-clay">{error}</p> : null}
      {readout ? <p className="text-sm break-words text-paper">{readout}</p> : null}
      <button type="submit" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={current.sign ? lock.locked : reading}>
        {current.sign ? "Review call" : reading ? "Reading…" : "Read"}
      </button>
      <ReceiptCard />
    </form>
  );
}

function ReceiptCard() {
  const row = useActivity((state) => state.rows.find((item) => item.direction === "contract"));
  const code = row?.code ?? (row?.status === "confirmed" ? "0x1" : row?.status === "failed" ? "0x0" : null);
  const label = !row ? "No contract receipt yet." : row.status === "confirmed" ? `status ${code ?? "0x1"} Success` : row.status === "failed" ? `status ${code ?? "0x0"} Revert` : "status Pending";
  return (
    <section className="rounded-3xl border border-line bg-ink px-4 py-3 text-sm">
      <h3 className="font-medium text-paper">Recent execution receipt</h3>
      <p className="mt-2 whitespace-pre-wrap break-all text-muted">
        {label}
        {row?.gasUsed ? `\ngasUsed ${row.gasUsed}` : ""}
        {row?.height != null ? `\nblock ${row.height}` : ""}
        {row?.hash ? `\ntx ${row.hash}` : ""}
        {row?.error ? `\n${row.error}` : ""}
        {row?.logs ? `\nlogs ${row.logs}` : ""}
      </p>
    </section>
  );
}

function hexToId(value: string): Uint8Array {
  const clean = value.trim().toLowerCase().replace(/^0x/, "");
  if (!/^[0-9a-f]{64}$/.test(clean)) throw new Error("Record id must be 64 hex characters.");
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i += 1) out[i] = Number.parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function Field({ label, value, onChange, mono }: { label: string; value: string; onChange: (value: string) => void; mono?: boolean }) {
  return (
    <label className="grid gap-1 text-sm text-muted">
      {label}
      <input className={`min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper ${mono ? "font-mono text-sm" : "tabular-nums"}`} value={value} onChange={(event) => onChange(event.target.value)} autoCapitalize="none" spellCheck={false} />
    </label>
  );
}

