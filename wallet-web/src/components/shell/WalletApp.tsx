import { useEffect, useState } from "react";
import { formatAtoms, formatKvnc } from "@/core/format";
import type { TxPreview } from "@/core/session";
import { PRESETS, chainName, presetById } from "@/lib/networks";
import { broadcast, indexerLogs, probeNode, readAccount, receipt } from "@/services/rpc";
import { signReviewed } from "@/services/wallet";
import { addAccount, forgetDevice, loadVault, lock, selectAccount } from "@/services/wallet";
import { useActivity } from "@/stores/activity";
import { signingLock, useNetwork } from "@/stores/network";
import { useSession } from "@/stores/session";
import { ActivityList } from "@/components/activity/ActivityList";
import { ConfirmTx } from "@/components/confirm/ConfirmTx";
import { Contracts } from "@/components/contracts/Contracts";
import { Onboarding } from "@/components/onboarding/Onboarding";
import { Receive } from "@/components/receive/Receive";
import { SendForm } from "@/components/send/SendForm";
import { toast } from "sonner";

type Tab = "home" | "send" | "receive" | "activity" | "contracts";

export function WalletApp() {
  const [ready, setReady] = useState(true);
  const [tab, setTab] = useState<Tab>("home");
  const [preview, setPreview] = useState<TxPreview | null>(null);
  const [signing, setSigning] = useState(false);
  const session = useSession();
  const net = useNetwork();
  const lockState = signingLock(net);
  const account = session.accounts[session.active];

  useEffect(() => {
    useSession.getState().hydrate(loadVault());
    useActivity.getState().hydrate();
    setReady(true);
  }, []);

  useEffect(() => {
    if (!ready) return;
    let stop = false;
    const run = async () => {
      useNetwork.getState().setProbing(true);
      const preset = presetById(useNetwork.getState().presetId);
      const probe = await probeNode(preset.rpcUrl);
      if (!stop) useNetwork.getState().applyProbe(probe);
    };
    void run();
    const timer = window.setInterval(() => void run(), 20000);
    return () => {
      stop = true;
      window.clearInterval(timer);
    };
  }, [ready, net.presetId]);

  useEffect(() => {
    if (!ready || session.status !== "unlocked" || !account || !net.online) return;
    const preset = presetById(net.presetId);
    void readAccount(preset.rpcUrl, account.address).then((state) => {
      useNetwork.getState().setAccountState(state.balance, state.nonce, state.fee);
    });
  }, [ready, session.status, account?.address, net.online, net.presetId, net.probedChainId]);

  useEffect(() => {
    if (session.status !== "unlocked") return;
    const timer = window.setTimeout(() => lock(), 5 * 60 * 1000);
    return () => window.clearTimeout(timer);
  }, [session.status, tab]);

  useEffect(() => {
    if (!ready || !net.online) return;
    const pending = useActivity.getState().rows.filter((row) => row.status === "pending");
    if (pending.length === 0) return;
    const preset = presetById(net.presetId);
    void Promise.all(
      pending.map(async (row) => {
        const result = await receipt(preset.rpcUrl, row.hash);
        if (result && result.status !== "pending") useActivity.getState().mark(row.hash, result.status, result.height);
      }),
    );
  }, [ready, net.online, net.presetId, tab]);

  useEffect(() => {
    if (!net.indexerUrl.trim()) {
      useActivity.getState().setIndexerNote("Outgoing transactions only");
      return;
    }
    const url = net.indexerUrl.includes("?") ? net.indexerUrl : `${net.indexerUrl}?limit=20`;
    void indexerLogs(url)
      .then(() => useActivity.getState().setIndexerNote("Indexer connected. Incoming rows appear when the feed includes this address."))
      .catch(() => useActivity.getState().setIndexerNote("Outgoing transactions only. The indexer did not respond."));
  }, [net.indexerUrl]);

  async function confirm() {
    if (!preview || lockState.locked) return;
    setSigning(true);
    try {
      const signed = signReviewed(preview);
      const preset = presetById(net.presetId);
      const hash = await broadcast(preset.rpcUrl, signed.raw);
      useActivity.getState().push({
        hash: hash || signed.hash,
        status: "pending",
        direction: preview.kind === "transfer" ? "sent" : "contract",
        amountAtoms: preview.kind === "transfer" ? preview.amountAtoms : preview.valueAtoms,
        feeAtoms: preview.feeAtoms,
        counterparty: preview.counterparty,
        chainId: preview.chainId,
        createdAt: Date.now(),
        height: null,
        note: preview.method,
      });
      setPreview(null);
      setTab("activity");
      toast("Transaction broadcast");
    } catch (error) {
      toast(error instanceof Error ? error.message : "Broadcast failed");
    } finally {
      setSigning(false);
    }
  }

  if (session.status !== "unlocked") {
    return (
      <main className="grid min-h-screen place-items-center bg-ink text-paper">
        <Onboarding key={session.status} mode={session.status === "locked" ? "unlock" : "choose"} />
      </main>
    );
  }

  const dot = lockState.locked ? "bg-clay" : "bg-sage";

  return (
    <main className="min-h-screen bg-ink text-paper">
      <div className="mx-auto grid min-h-screen w-full max-w-5xl md:grid-cols-[240px_1fr]">
        <aside className="border-b border-line px-5 py-6 md:border-r md:border-b-0">
          <p className="text-xs tracking-[0.22em] text-copper uppercase">Kovanica</p>
          <h1 className="mt-2 text-2xl font-semibold">Wallet</h1>
          <div className="mt-6 flex items-center gap-2 text-sm">
            <span className={`h-2.5 w-2.5 rounded-full ${dot}`} />
            <span>{lockState.networkName}</span>
          </div>
          <p className="mt-2 text-xs text-muted">
            {net.probing ? "Probing…" : net.online ? `height ${net.committedHeight ?? "—"} · ${net.latencyMs ?? "—"} ms` : (net.error ?? "Offline")}
          </p>
          {lockState.locked ? <p className="mt-3 text-sm text-clay">{lockState.reason}</p> : null}
          <div className="mt-4 grid grid-cols-2 gap-2">
            {PRESETS.map((preset) => (
              <button
                key={preset.id}
                type="button"
                className={`min-h-11 rounded-full border text-xs ${net.presetId === preset.id ? "border-copper text-copper" : "border-line text-muted"}`}
                onClick={() => useNetwork.getState().selectPreset(preset.id)}
              >
                {preset.label}
              </button>
            ))}
          </div>
          <nav className="mt-6 grid grid-cols-3 gap-2 md:grid-cols-1">
            {(["home", "send", "receive", "activity", "contracts"] as const).map((item) => (
              <button key={item} type="button" className={`min-h-11 rounded-full px-3 text-left capitalize ${tab === item ? "bg-surface text-paper" : "text-muted"}`} onClick={() => setTab(item)}>
                {item}
              </button>
            ))}
          </nav>
        </aside>
        <section className="px-5 py-6">
          {tab === "home" ? (
            <div className="grid gap-5">
              <p className="text-sm text-muted">{account?.pathLabel}</p>
              {session.kind === "legacy" ? <p className="w-fit rounded-full border border-copper px-3 py-1 text-sm text-copper">Legacy account</p> : null}
              <p className="text-5xl font-semibold tabular-nums">{net.balanceAtoms == null ? "—" : formatKvnc(net.balanceAtoms)}</p>
              <p className="text-muted">{net.balanceAtoms == null ? "Balance unavailable" : `${formatAtoms(net.balanceAtoms)} atoms`}</p>
              <p className="mono text-sm break-all">{account?.address}</p>
              <div className="flex flex-wrap gap-2">
                {session.accounts.map((item, index) => (
                  <button key={item.address} type="button" className={`min-h-11 rounded-full border px-4 text-sm ${index === session.active ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => selectAccount(index)}>
                    {item.pathLabel === "Legacy account" ? "Legacy" : `Account ${item.account}`}
                  </button>
                ))}
                {session.kind === "hd" ? (
                  <button type="button" className="min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={() => void addAccount()}>
                    Add account
                  </button>
                ) : null}
              </div>
              <div className="flex gap-3">
                <button type="button" className="min-h-11 rounded-full bg-copper px-5 font-medium text-copper-ink" onClick={() => setTab("send")}>Send</button>
                <button type="button" className="min-h-11 rounded-full border border-line px-5" onClick={() => setTab("receive")}>Receive</button>
                <button type="button" className="min-h-11 rounded-full border border-line px-5" onClick={() => lock()}>Lock</button>
              </div>
              <label className="grid gap-1 text-sm text-muted">
                Indexer URL, optional
                <input className="min-h-11 rounded-2xl border border-line bg-surface px-4 text-paper" value={net.indexerUrl} onChange={(e) => useNetwork.getState().setIndexerUrl(e.target.value)} placeholder="https://indexer.kovanica.online/logs" />
              </label>
              <button type="button" className="w-fit text-sm text-clay" onClick={() => { forgetDevice(); }}>
                Remove wallet from this browser
              </button>
              <p className="text-xs text-muted">Chain {net.probedChainId ?? "—"} · expected {presetById(net.presetId).expectedChainId ?? "reported"} · {chainName(net.probedChainId)}</p>
            </div>
          ) : null}
          {tab === "send" ? <SendForm onReview={setPreview} /> : null}
          {tab === "receive" && account ? <Receive address={account.address} pathLabel={account.pathLabel} /> : null}
          {tab === "activity" ? <ActivityList /> : null}
          {tab === "contracts" ? <Contracts onReview={setPreview} /> : null}
        </section>
      </div>
      {preview ? <ConfirmTx preview={preview} busy={signing} onCancel={() => setPreview(null)} onConfirm={() => void confirm()} /> : null}
    </main>
  );
}
