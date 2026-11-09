import { useState } from "react";
import { checkAddress } from "@/core/address";
import { presetById } from "@/lib/networks";
import { readContract, readNodeWatch } from "@/services/rpc";
import { useNetwork } from "@/stores/network";
import { useSession } from "@/stores/session";
import { loadWatch, removeWatch, saveWatch, type WatchItem, type WatchKind } from "@/stores/watch";

const KINDS: WatchKind[] = ["token", "vault", "htlc", "multisig"];
const READS: Record<WatchKind, string> = {
  token: "balanceOf",
  vault: "vested",
  htlc: "locks",
  multisig: "threshold",
};

export function Watchlist({ onSend, onContract }: { onSend: (address: string) => void; onContract: (item: WatchItem) => void }) {
  const net = useNetwork();
  const account = useSession((s) => s.accounts[s.active]);
  const chainId = net.probedChainId ?? presetById(net.presetId).expectedChainId;
  const [items, setItems] = useState<WatchItem[]>(() => loadWatch());
  const [kind, setKind] = useState<WatchKind>("token");
  const [label, setLabel] = useState("");
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [reads, setReads] = useState<Record<string, { call: string; storage: string }>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [miner, setMiner] = useState(account?.address ?? "");
  const [nodeRead, setNodeRead] = useState<{ dag: string; template: string; header: string } | null>(null);
  const [nodeBusy, setNodeBusy] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);
  const reportedChain = nodeRead ? fieldIn(nodeRead, ["chainId", "chain_id"]) : null;
  const hybrid = chainId === 5 || reportedChain === "5";

  async function refresh(item: WatchItem) {
    if (!net.online) return setError("The node is not responding, so this contract cannot be read.");
    setBusy(item.id);
    setError(null);
    try {
      const result = await readContract(presetById(net.presetId).rpcUrl, item.address, READS[item.kind], account?.publicKey ?? "");
      setReads((current) => ({ ...current, [item.id]: result }));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not read the contract");
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="grid gap-4">
      <div>
        <h2 className="text-2xl font-semibold">Watch</h2>
        <p className="mt-2 text-sm text-muted">Read-only. Token rows ask `balanceOf`. Vault, HTLC, and multisig ask `vested`, `locks`, and `threshold`, plus `kovanica_getStorage` at your account key.</p>
      </div>
      <form
        className="grid gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          setError(null);
          try {
            saveWatch(kind, label, address, chainId);
            setItems(loadWatch());
            setLabel("");
            setAddress("");
          } catch (err) {
            setError(err instanceof Error ? err.message : "Could not watch that address");
          }
        }}
      >
        <div className="flex flex-wrap gap-2">
          {KINDS.map((item) => (
            <button key={item} type="button" className={`min-h-11 rounded-full border px-4 text-sm capitalize ${kind === item ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => setKind(item)}>
              {item}
            </button>
          ))}
        </div>
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Label" />
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 font-mono text-sm text-paper" value={address} onChange={(e) => setAddress(e.target.value)} placeholder="kvnc…dag" autoCapitalize="none" spellCheck={false} />
        {address.trim() ? <p className={checkAddress(address, chainId).ok ? "text-sm text-sage" : "text-sm text-clay"}>{checkAddress(address, chainId).message}</p> : null}
        <button type="submit" className="min-h-11 rounded-full bg-copper font-medium text-copper-ink">Watch contract</button>
      </form>
      {error ? <p className="text-sm text-clay">{error}</p> : null}
      {items.map((item) => (
        <article key={item.id} className="rounded-card border border-line bg-ink p-4">
          <div className="flex items-start justify-between gap-3">
            <div>
              <p className="text-sm text-copper capitalize">{item.kind}</p>
              <p className="text-lg text-paper">{item.label}</p>
              <p className="mono mt-1 text-xs break-all text-muted">{item.address}</p>
            </div>
            <button type="button" className="min-h-11 text-sm text-clay" onClick={() => setItems(removeWatch(item.id))}>Remove</button>
          </div>
          {reads[item.id] ? (
            <dl className="mt-3 grid gap-2 text-sm">
              <div><dt className="text-muted">Call {READS[item.kind]}</dt><dd className="mono break-all text-paper">{reads[item.id].call}</dd></div>
              <div><dt className="text-muted">Storage</dt><dd className="mono break-all text-paper">{reads[item.id].storage}</dd></div>
            </dl>
          ) : <p className="mt-3 text-sm text-muted">Not read yet.</p>}
          <div className="mt-3 flex flex-wrap gap-2">
            <button type="button" className="min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={() => void refresh(item)} disabled={busy === item.id}>
              {busy === item.id ? "Reading…" : "Read"}
            </button>
            <button type="button" className="min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={() => onSend(item.address)}>Send</button>
            <button type="button" className="min-h-11 rounded-full border border-copper px-4 text-sm text-copper" onClick={() => onContract(item)}>Use in contracts</button>
          </div>
        </article>
      ))}
      <section className="rounded-card border border-line bg-ink p-4">
        <h3 className="text-lg text-paper">Miner / node</h3>
        {hybrid ? <p className="mt-3 inline-flex min-h-8 items-center rounded-full border border-copper px-3 text-xs text-copper">Hybrid PoW + GHOSTDAG (K=5, 4s)</p> : null}
        <p className="mt-2 text-sm text-muted">Chain 5 is Hybrid PoW + GHOSTDAG. K is 5 and the target block time is 4 seconds. target_bits and work are shown only when the node includes them. This screen does not recompute KovanicaHash. The phone and the desktop console do. A signed air-gap QR is scanned from Confirm, checked against this chain, and broadcast with kovanica_sendRawTransaction.</p>
        <label className="mt-3 grid gap-1 text-sm text-muted">
          Watched miner
          <input className="min-h-11 rounded-2xl border border-line bg-surface px-4 font-mono text-sm text-paper" value={miner} onChange={(e) => setMiner(e.target.value)} spellCheck={false} autoCapitalize="none" />
        </label>
        <button
          type="button"
          className="mt-3 min-h-11 rounded-full border border-line px-4 text-sm text-paper"
          disabled={nodeBusy}
          onClick={() => {
            if (!net.online) return setError("The node is not responding, so the miner view cannot be read.");
            setNodeBusy(true);
            setError(null);
            void readNodeWatch(presetById(net.presetId).rpcUrl)
              .then(setNodeRead)
              .catch((err: unknown) => setError(err instanceof Error ? err.message : "Could not read the node"))
              .finally(() => setNodeBusy(false));
          }}
        >
          {nodeBusy ? "Reading…" : "Read node"}
        </button>
        {nodeRead ? (
          <div className="mt-3 grid gap-3 text-sm">
            <p className="text-paper">GHOSTDAG K=5 · target 4 s · pruning 14,400 rounds · cache 64 MiB</p>
            <p className="text-paper">target_bits {fieldIn(nodeRead, ["target_bits", "targetBits"]) ?? "not in this response"}</p>
            <p className="text-paper">work {fieldIn(nodeRead, ["work", "blue_work", "blueWork"]) ?? "not in this response"}</p>
            <CopyRow label="powHash" value={fieldIn(nodeRead, ["powHash", "pow_hash"])} copied={copied} onCopy={(value) => void copyText("powHash", value, setCopied, setError)} />
            <CopyRow label="tip round" value={fieldIn(nodeRead, ["round", "tipRound", "tip_round"])} copied={copied} onCopy={(value) => void copyText("tip round", value, setCopied, setError)} />
            <p className="text-muted">{miner && (nodeRead.dag.includes(miner) || nodeRead.template.includes(miner) || nodeRead.header.includes(miner)) ? "The watched miner appears in this response." : "The watched miner does not appear in this response."}</p>
            <div>
              <p className="text-muted">DAG info</p>
              <pre className="mono mt-1 max-h-40 overflow-auto text-xs break-all whitespace-pre-wrap text-paper">{nodeRead.dag}</pre>
            </div>
            <div>
              <p className="text-muted">Block template</p>
              <pre className="mono mt-1 max-h-40 overflow-auto text-xs break-all whitespace-pre-wrap text-paper">{nodeRead.template}</pre>
            </div>
          </div>
        ) : null}
      </section>
    </div>
  );
}

function CopyRow({ label, value, copied, onCopy }: { label: string; value: string | null; copied: string | null; onCopy: (value: string) => void }) {
  if (!value) return <p className="text-paper">{label} not in this response</p>;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <p className="mono text-paper break-all">{label} {value}</p>
      <button type="button" className="min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={() => onCopy(value)}>
        {copied === label ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

async function copyText(label: string, value: string, setCopied: (label: string) => void, setError: (message: string) => void) {
  try {
    await navigator.clipboard.writeText(value);
    setCopied(label);
  } catch {
    setError("Could not copy that value.");
  }
}

function fieldIn(read: { dag: string; template: string; header: string }, keys: string[]): string | null {
  for (const text of [read.header, read.dag, read.template]) {
    const found = findField(text, keys);
    if (found) return found;
  }
  return null;
}

function findField(text: string, keys: string[]): string | null {
  try {
    return walk(JSON.parse(text), keys);
  } catch {
    return null;
  }
}

function walk(value: unknown, keys: string[]): string | null {
  if (!value || typeof value !== "object") return null;
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = walk(item, keys);
      if (found) return found;
    }
    return null;
  }
  const record = value as Record<string, unknown>;
  for (const key of keys) {
    const child = record[key];
    if (typeof child === "string" || typeof child === "number") return String(child);
  }
  for (const child of Object.values(record)) {
    const found = walk(child, keys);
    if (found) return found;
  }
  return null;
}
