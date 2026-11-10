import { useEffect, useRef, useState } from "react";
import { formatAtoms, formatKvnc } from "@/core/format";
import type { TxPreview } from "@/core/session";
import { AddressBook } from "@/components/contacts/AddressBook";
import { chainName, presetById } from "@/lib/networks";
import { broadcast, indexerLogs, probeNode, readAccount, receipt } from "@/services/rpc";
import { onWalletLock, touchSession } from "@/services/tauri";
import { addAccount, clearBackup, forgetDevice, isDesktopShell, loadVault, lock, lockUi, refreshDesktopSession, selectAccount, signReviewed, subscribeBackup } from "@/services/wallet";
import { useActivity } from "@/stores/activity";
import { savedNetworkChoice, signingLock, useNetwork } from "@/stores/network";
import { useSession } from "@/stores/session";
import { ActivityList } from "@/components/activity/ActivityList";
import { ConfirmTx, type BroadcastNotice } from "@/components/confirm/ConfirmTx";
import { Contracts } from "@/components/contracts/Contracts";
import { Onboarding } from "@/components/onboarding/Onboarding";
import { Receive } from "@/components/receive/Receive";
import { SendForm } from "@/components/send/SendForm";
import { FaucetButton } from "@/components/home/FaucetButton";
import { NetworkChoices } from "@/components/settings/NetworkChoices";
import { SettingsDrawer } from "@/components/settings/SettingsDrawer";
import { Watchlist } from "@/components/watch/Watchlist";
import { useSettings } from "@/stores/settings";
import { useIdle } from "@/stores/idle";
import { remainingMs } from "@/core/idle";
import type { WatchKind } from "@/stores/watch";

import { toast } from "sonner";

type Tab = "home" | "send" | "receive" | "activity" | "contracts" | "watch";

export function WalletApp() {
  const [ready, setReady] = useState(true);
  const [tab, setTab] = useState<Tab>("home");
  const [preview, setPreview] = useState<TxPreview | null>(null);
  const [notice, setNotice] = useState<BroadcastNotice>({ phase: "idle" });
  const [quoteTick, setQuoteTick] = useState(0);
  const [host, setHost] = useState("");
  const [signing, setSigning] = useState(false);
  const warned = useRef("");
  const [backup, setBackup] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sendTo, setSendTo] = useState("");
  const [contractPrefill, setContractPrefill] = useState<{ address: string; kind: WatchKind } | null>(null);
  const lockMinutes = useSettings((s) => s.lockMinutes);
  const activityAt = useIdle((s) => s.at);
  const bumpIdle = useIdle((s) => s.bump);
  const session = useSession();
  const net = useNetwork();
  const lockState = signingLock(net);
  const account = session.accounts[session.active];

  useEffect(() => {
    const stopBackup = subscribeBackup(setBackup);
    let stopLock = () => {};
    if (isDesktopShell()) {
      void refreshDesktopSession();
      void onWalletLock(() => lockUi()).then((stop) => {
        stopLock = stop;
      });
      const touch = () => {
        void touchSession().catch(() => undefined);
      };
      window.addEventListener("pointerdown", touch);
      useActivity.getState().hydrate();
      setReady(true);
      return () => {
        stopBackup();
        stopLock();
        window.removeEventListener("pointerdown", touch);
      };
    } else {
      useSession.getState().hydrate(loadVault());
    }
    useActivity.getState().hydrate();
    const saved = savedNetworkChoice();
    if (saved) useNetwork.getState().setRpcUrl(saved.rpcUrl, saved.presetId);
    setReady(true);
    return () => {
      stopBackup();
      stopLock();
    };
  }, []);

  useEffect(() => {
    if (!ready) return;
    let stop = false;
    const run = async () => {
      useNetwork.getState().setProbing(true);
      const rpcUrl = useNetwork.getState().rpcUrl;
      const probe = await probeNode(rpcUrl);
      if (stop) return;
      useNetwork.getState().applyProbe(probe);
      if (!probe.online) {
        const message = probe.error ?? "The node did not respond.";
        if (warned.current !== message) {
          warned.current = message;
          toast(`${message} You can switch to the testnet fallback or enter a host.`);
        }
      } else {
        warned.current = "";
      }
    };
    void run();
    const timer = window.setInterval(() => void run(), 20000);
    return () => {
      stop = true;
      window.clearInterval(timer);
    };
  }, [ready, net.presetId, net.rpcUrl]);

  useEffect(() => {
    if (!ready || session.status !== "unlocked" || !account || !net.online) return;
    void readAccount(net.rpcUrl, account.address).then((state) => {
      useNetwork.getState().setAccountState(state.balance, state.nonce, state.fee);
    });
  }, [ready, session.status, account?.address, net.online, net.presetId, net.probedChainId, net.rpcUrl]);

  useEffect(() => {
    const mark = () => bumpIdle();
    window.addEventListener("pointerdown", mark);
    window.addEventListener("keydown", mark);
    return () => {
      window.removeEventListener("pointerdown", mark);
      window.removeEventListener("keydown", mark);
    };
  }, [bumpIdle]);

  useEffect(() => {
    if (session.status !== "unlocked") return;
    const timer = window.setTimeout(() => void lock(), remainingMs(lockMinutes, activityAt, Date.now()));
    return () => window.clearTimeout(timer);
  }, [session.status, lockMinutes, activityAt]);

  useEffect(() => {
    if (!ready || !net.online) return;
    let stop = false;
    const run = async () => {
      const pending = useActivity.getState().rows.filter((row) => row.status === "pending");
      if (pending.length === 0) return;
      const rpcUrl = useNetwork.getState().rpcUrl;
      await Promise.all(
        pending.map(async (row) => {
          const result = await receipt(rpcUrl, row.hash);
          if (!stop && result && result.status !== "pending") {
            useActivity.getState().mark(row.hash, result.status, result.height, {
              gasUsed: result.gasUsed,
              code: result.code,
              error: result.error,
              logs: result.logs,
            });
          }
        }),
      );
    };
    void run();
    const timer = window.setInterval(() => void run(), 5_000);
    return () => {
      stop = true;
      window.clearInterval(timer);
    };
  }, [ready, net.online, net.presetId, net.rpcUrl]);

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

  async function publish(raw: string, hash: string) {
    if (!preview) return;
    const preset = presetById(net.presetId);
    setNotice({ phase: "sending", label: preset.label });
    const broadcastHash = await broadcast(net.rpcUrl, raw);
    const shown = broadcastHash || hash;
    useActivity.getState().push({
      hash: shown,
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
    setNotice({ phase: "ok", label: preset.label, hash: shown });
  }

  async function confirm() {
    if (!preview || lockState.locked) return;
    setSigning(true);
    try {
      const signed = await signReviewed(preview);
      await publish(signed.raw, signed.hash);
    } catch (error) {
      setNotice({ phase: "error", message: error instanceof Error ? error.message : "Broadcast failed" });
    } finally {
      setSigning(false);
    }
  }

  async function importSigned(signed: { raw: string; hash: string }) {
    if (!preview || lockState.locked) return;
    setSigning(true);
    try {
      await publish(signed.raw, signed.hash);
    } catch (error) {
      setNotice({ phase: "error", message: error instanceof Error ? error.message : "Broadcast failed" });
    } finally {
      setSigning(false);
    }
  }

  function editDraft() {
    setNotice({ phase: "idle" });
    setPreview(null);
  }

  async function requote() {
    try {
      if (account) {
        const state = await readAccount(net.rpcUrl, account.address);
        useNetwork.getState().setAccountState(state.balance, state.nonce, state.fee);
      }
      setQuoteTick((tick) => tick + 1);
      setNotice({ phase: "idle" });
      setPreview(null);
    } catch (error) {
      setNotice({ phase: "error", message: error instanceof Error ? error.message : "Could not re-quote the fee" });
    }
  }

  function finishBroadcast() {
    setNotice({ phase: "idle" });
    setPreview(null);
    setTab("activity");
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
          <div className="mt-4">
            <NetworkChoices />
          </div>
          <nav className="mt-6 grid grid-cols-3 gap-2 md:grid-cols-1">
            {(["home", "contracts", "send", "activity", "receive", "watch"] as const).map((item) => (
              <button key={item} type="button" className={`min-h-11 rounded-full px-3 text-left capitalize ${tab === item ? "bg-surface text-paper" : "text-muted"}`} onClick={() => setTab(item)}>
                {item}
              </button>
            ))}
          </nav>
          <button type="button" className="mt-4 min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={() => setSettingsOpen(true)}>
            Settings
          </button>
        </aside>
        <section className="px-5 py-6">
          <div className="mb-5 flex gap-2 overflow-x-auto">
            {session.accounts.map((item, index) => (
              <button key={`${item.pathLabel}-${index}`} type="button" className={`min-h-11 shrink-0 rounded-full border px-4 text-left text-sm ${index === session.active ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => selectAccount(index)}>
                <span className="block">{item.pathLabel === "Legacy account" ? "Legacy account" : `Account ${item.account}`}</span>
                <span className="block text-xs">{item.pathLabel}</span>
              </button>
            ))}
            {session.kind === "hd" ? (
              <button type="button" className="min-h-11 shrink-0 rounded-full border border-line px-4 text-sm text-paper" onClick={() => void addAccount().catch((error: unknown) => toast(error instanceof Error ? error.message : "Could not add an account"))}>
                Add account
              </button>
            ) : null}
          </div>
          {tab === "home" ? (
            <div className="grid gap-5">
              <p className="text-sm text-muted">{account?.pathLabel}</p>
              {session.kind === "legacy" ? <p className="w-fit rounded-full border border-copper px-3 py-1 text-sm text-copper">Legacy account</p> : null}
              <p className="text-5xl font-semibold tabular-nums">{net.balanceAtoms == null ? "—" : formatKvnc(net.balanceAtoms)}</p>
              <p className="text-muted">{net.balanceAtoms == null ? "Balance unavailable" : `${formatAtoms(net.balanceAtoms)} atoms`}</p>
              {account ? <FaucetButton address={account.address} /> : null}
              <p className="mono text-sm break-all">{account?.address}</p>
              <div className="flex gap-3">
                <button type="button" className="min-h-11 rounded-full bg-copper px-5 font-medium text-copper-ink" onClick={() => setTab("send")}>Send</button>
                <button type="button" className="min-h-11 rounded-full border border-line px-5" onClick={() => setTab("receive")}>Receive</button>
                <button type="button" className="min-h-11 rounded-full border border-line px-5" onClick={() => lock()}>Lock</button>
              </div>
              <label className="grid gap-1 text-sm text-muted">
                Indexer URL, optional
                <input className="min-h-11 rounded-2xl border border-line bg-surface px-4 text-paper" value={net.indexerUrl} onChange={(e) => useNetwork.getState().setIndexerUrl(e.target.value)} placeholder="https://indexer.kovanica.online/logs" />
              </label>
              <AddressBook />
              <button type="button" className="w-fit text-sm text-clay" onClick={() => { forgetDevice(); }}>
                {isDesktopShell() ? "Lock the desktop wallet" : "Remove wallet from this browser"}
              </button>
              <p className="text-xs text-muted">Chain {net.probedChainId ?? "—"} · expected {presetById(net.presetId).expectedChainId ?? "reported"} · {chainName(net.probedChainId)}</p>
            </div>
          ) : null}
          {tab === "send" ? <SendForm onReview={(next) => { setNotice({ phase: "idle" }); setPreview(next); }} prefillTo={sendTo} quoteTick={quoteTick} /> : null}
          {tab === "receive" && account ? <Receive address={account.address} pathLabel={account.pathLabel} /> : null}
          {tab === "activity" ? <ActivityList /> : null}
          {tab === "contracts" ? <Contracts onReview={(next) => { setNotice({ phase: "idle" }); setPreview(next); }} prefill={contractPrefill} ownerAddress={account?.address ?? ""} /> : null}
          {tab === "watch" ? (
            <Watchlist
              onSend={(address) => { setSendTo(address); setTab("send"); }}
              onContract={(item) => { setContractPrefill({ address: item.address, kind: item.kind }); setTab("contracts"); }}
            />
          ) : null}
        </section>
      </div>
      {preview ? <ConfirmTx preview={preview} busy={signing} notice={notice} onCancel={editDraft} onConfirm={() => void confirm()} onImport={(signed) => void importSigned(signed)} onEdit={editDraft} onRequote={() => void requote()} onDone={finishBroadcast} /> : null}
      {settingsOpen ? <SettingsDrawer onClose={() => setSettingsOpen(false)} /> : null}
      {backup ? (
        <div className="fixed inset-0 z-40 grid place-items-center bg-ink/90 p-5">
          <div className="w-full max-w-lg rounded-card border border-line bg-surface p-5">
            <h2 className="text-2xl font-semibold">Write down this phrase</h2>
            <p className="mt-2 text-sm text-muted">This is the only time the desktop signer shows it. It is cleared when the wallet locks.</p>
            <p className="mono mt-4 text-sm break-words text-paper">{backup}</p>
            <button type="button" className="mt-5 min-h-12 w-full rounded-full bg-copper font-medium text-copper-ink" onClick={() => clearBackup()}>
              I wrote it down
            </button>
          </div>
        </div>
      ) : null}
    </main>
  );
}
