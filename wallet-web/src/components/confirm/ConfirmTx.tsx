import { useCallback, useEffect, useRef, useState } from "react";
import { acceptSigned, exportUnsigned } from "@/core/airgap";
import { callNeed, leadingAtoms, shortfall } from "@/core/history-row";
import type { TxPreview } from "@/core/session";
import { reviewRows } from "@/services/tx";
import { useNetwork } from "@/stores/network";
import { FaucetButton } from "@/components/home/FaucetButton";
import { toast } from "sonner";

export type BroadcastNotice =
  | { phase: "idle" }
  | { phase: "sending"; label: string }
  | { phase: "ok"; label: string; hash: string }
  | { phase: "error"; message: string };

export function ConfirmTx({
  preview,
  busy,
  notice,
  onCancel,
  onConfirm,
  onImport,
  onEdit,
  onRequote,
  onDone,
}: {
  preview: TxPreview;
  busy: boolean;
  notice: BroadcastNotice;
  onCancel: () => void;
  onConfirm: () => void;
  onImport: (signed: { raw: string; hash: string }) => void;
  onEdit: () => void;
  onRequote: () => void;
  onDone: () => void;
}) {
  const rows = reviewRows(preview);
  const [qr, setQr] = useState<string | null>(null);
  const [payload, setPayload] = useState<string | null>(null);
  const [incoming, setIncoming] = useState("");
  const unsigned = JSON.stringify(exportUnsigned(preview));
  const balance = useNetwork((state) => state.balanceAtoms);
  const pull = preview.kind === "call" && (preview.method === "htlc_create" || preview.method === "vault_create")
    ? leadingAtoms(preview.details.find((row) => row.label === "Amount")?.value ?? "0")
    : "0";
  const need = preview.kind === "call" ? callNeed(preview.method, preview.valueAtoms, preview.feeAtoms, pull) : 0n;
  const missing = balance == null ? 0n : shortfall(balance, need);
  const takeSigned = useCallback((text: string) => {
    try {
      const signed = acceptSigned(text);
      if (signed.doc.chainId !== preview.chainId) {
        toast(`This signature is for chain ${signed.doc.chainId}. The review is chain ${preview.chainId}.`);
        return;
      }
      onImport({ raw: signed.raw, hash: signed.hash });
    } catch (error) {
      toast(error instanceof Error ? error.message : "That payload was rejected");
    }
  }, [onImport, preview.chainId]);

  return (
    <div className="fixed inset-0 z-30 flex items-end justify-center bg-ink/80 p-4 sm:items-center" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <div className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-card border border-line bg-surface p-5 shadow-2xl">
        <p className="text-xs tracking-[0.18em] text-copper uppercase">Review before signing</p>
        <h2 id="confirm-title" className="mt-2 text-2xl font-semibold text-paper">
          Confirm transaction
        </h2>
        <p className="mt-2 text-sm text-muted">Nothing is signed until you confirm. The chain id below is the one the node reported.</p>
        <BroadcastBanner notice={notice} busy={busy} onEdit={onEdit} onRequote={onRequote} onDone={onDone} />
        <dl className="mt-5 space-y-3">
          {rows.map((row) => (
            <div key={row.label} className="grid gap-1 border-b border-line pb-3">
              <dt className="text-xs tracking-wide text-muted uppercase">{row.label}</dt>
              <dd className={row.mono ? "mono text-sm break-all text-paper" : "text-sm text-paper"}>{row.value}</dd>
            </div>
          ))}
        </dl>
        {preview.kind === "call" && missing > 0n ? (
          <div className="mt-4 rounded-2xl border border-clay px-4 py-3 text-sm text-clay">
            <p>Balance is short by {missing.toString()} atoms for this call value and fee.</p>
            <div className="mt-3">
              <FaucetButton address={preview.sender} onlyHybrid />
            </div>
          </div>
        ) : null}
        <div className="mt-5 grid grid-cols-2 gap-3">
          <button type="button" className="min-h-11 rounded-full border border-line text-paper" onClick={onCancel} disabled={busy}>
            Back
          </button>
          <button type="button" className="min-h-11 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" onClick={onConfirm} disabled={busy}>
            {busy ? "Signing…" : "Confirm & Sign"}
          </button>
        </div>
        <div className="mt-5 border-t border-line pt-4">
          <p className="text-sm text-paper">Sign on another device</p>
          <p className="mt-1 text-sm text-muted">This QR is the unsigned preview. The phone signs it offline and you bring the signed payload back. The seed never crosses.</p>
          <button
            type="button"
            className="mt-3 min-h-11 rounded-full border border-line px-4 text-sm text-paper"
            onClick={() => {
              setPayload(unsigned);
              void import("qrcode").then((mod) =>
                mod.toDataURL(unsigned, { margin: 1, width: 280, errorCorrectionLevel: "L", color: { dark: "#0C1412", light: "#F4F0E6" } }).then(setQr).catch(() => setQr(null)),
              );
            }}
          >
            Show air-gap QR
          </button>
          <button type="button" className="mt-3 ml-2 min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={() => downloadUnsigned(unsigned, preview.nonce)}>
            Download unsigned JSON
          </button>
          {payload ? (
            <div className="mt-3 grid gap-3">
              {qr ? <img src={qr} alt="Unsigned transaction QR" className="h-56 w-56 rounded-2xl bg-paper p-2" /> : <p className="text-sm text-muted">The payload is below if it does not fit in a QR.</p>}
              <textarea className="min-h-24 rounded-2xl border border-line bg-ink p-3 font-mono text-xs text-paper" readOnly value={payload} />
              <button type="button" className="min-h-11 w-fit text-sm text-copper" onClick={() => { void navigator.clipboard.writeText(payload); toast("Unsigned payload copied"); }}>
                Copy unsigned JSON
              </button>
            </div>
          ) : null}
          <ScanSigned busy={busy} onText={takeSigned} />
          <label className="mt-4 grid gap-1 text-sm text-muted">
            Load signed JSON
            <input
              className="text-sm text-paper"
              type="file"
              accept="application/json,.json"
              onChange={(event) => {
                const file = event.target.files?.[0];
                event.target.value = "";
                if (!file) return;
                void file.text().then((text) => takeSigned(text));
              }}
            />
          </label>
          <label className="mt-4 grid gap-1 text-sm text-muted">
            Signed payload from the phone
            <textarea className="min-h-24 rounded-2xl border border-line bg-ink p-3 font-mono text-xs text-paper" value={incoming} onChange={(e) => setIncoming(e.target.value)} placeholder='{"v":1,"role":"signed",...}' />
          </label>
          <button
            type="button"
            className="mt-3 min-h-11 rounded-full border border-copper px-4 text-sm text-copper disabled:opacity-40"
            disabled={busy || !incoming.trim()}
            onClick={() => takeSigned(incoming.trim())}
          >
            Import signed transaction
          </button>
        </div>
      </div>
    </div>
  );
}

function BroadcastBanner({
  notice,
  busy,
  onEdit,
  onRequote,
  onDone,
}: {
  notice: BroadcastNotice;
  busy: boolean;
  onEdit: () => void;
  onRequote: () => void;
  onDone: () => void;
}) {
  if (notice.phase === "idle") return null;
  if (notice.phase === "sending") {
    return <p className="mt-4 rounded-2xl border border-line bg-ink px-4 py-3 text-sm text-paper">Broadcasting to {notice.label}...</p>;
  }
  if (notice.phase === "ok") {
    return (
      <div className="mt-4 rounded-2xl border border-sage bg-ink px-4 py-3">
        <p className="text-sm text-sage">Transaction broadcast successful! TxHash: {notice.hash}</p>
        <button type="button" className="mt-3 min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={onDone}>Done</button>
      </div>
    );
  }
  return (
    <div className="mt-4 rounded-2xl border border-clay bg-ink px-4 py-3">
      <p className="text-sm text-clay">{notice.message}</p>
      <div className="mt-3 flex flex-wrap gap-2">
        <button type="button" className="min-h-11 rounded-full border border-line px-4 text-sm text-paper" onClick={onEdit} disabled={busy}>Edit</button>
        <button type="button" className="min-h-11 rounded-full border border-copper px-4 text-sm text-copper" onClick={onRequote} disabled={busy}>Re-quote</button>
      </div>
    </div>
  );
}

function downloadUnsigned(json: string, nonce: string) {
  const safe = nonce.replace(/[^\d]/g, "") || "0";
  const blob = new Blob([json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `kovanica-tx-unsigned-${safe}.json`;
  link.click();
  URL.revokeObjectURL(url);
}

type DetectedCode = { rawValue?: string };
type QrDetector = { detect: (source: CanvasImageSource) => Promise<DetectedCode[]> };

function ScanSigned({ busy, onText }: { busy: boolean; onText: (text: string) => void }) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    if (!open) return;
    let stop = false;
    let stream: MediaStream | null = null;
    const host = window as Window & { BarcodeDetector?: new (options: { formats: string[] }) => QrDetector };
    if (!host.BarcodeDetector || !navigator.mediaDevices?.getUserMedia) {
      setNote("This browser cannot read a QR from the camera. Paste the signed JSON instead.");
      return;
    }
    const detector = new host.BarcodeDetector({ formats: ["qr_code"] });
    void navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } }).then(async (next) => {
      stream = next;
      const video = videoRef.current;
      if (!video || stop) {
        next.getTracks().forEach((track) => track.stop());
        return;
      }
      video.srcObject = next;
      await video.play();
      const tick = async () => {
        if (stop || !videoRef.current) return;
        try {
          const codes = await detector.detect(videoRef.current);
          const raw = codes[0]?.rawValue;
          if (raw) {
            stop = true;
            setOpen(false);
            onText(raw);
            return;
          }
        } catch {
          // The frame is not ready yet.
        }
        if (!stop) requestAnimationFrame(() => void tick());
      };
      void tick();
    }).catch(() => setNote("The camera was not available."));
    return () => {
      stop = true;
      stream?.getTracks().forEach((track) => track.stop());
    };
  }, [open, onText]);

  return (
    <div className="mt-4">
      <button type="button" className="min-h-11 rounded-full border border-line px-4 text-sm text-paper" disabled={busy} onClick={() => { setNote(null); setOpen(true); }}>
        Scan signed QR
      </button>
      {note ? <p className="mt-2 text-sm text-clay">{note}</p> : null}
      {open ? (
        <div className="mt-3 grid gap-2">
          <video ref={videoRef} className="w-full max-w-xs rounded-2xl bg-ink" muted playsInline />
          <p className="text-sm text-muted">Point this at the phone's signed QR. A valid code is checked and sent with kovanica_sendRawTransaction.</p>
          <button type="button" className="min-h-11 w-fit text-sm text-copper" onClick={() => setOpen(false)}>Close camera</button>
        </div>
      ) : null}
    </div>
  );
}