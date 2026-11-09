import { useState } from "react";
import { acceptSigned, exportUnsigned } from "@/core/airgap";
import type { TxPreview } from "@/core/session";
import { reviewRows } from "@/services/tx";
import { toast } from "sonner";

export function ConfirmTx({
  preview,
  busy,
  onCancel,
  onConfirm,
  onImport,
}: {
  preview: TxPreview;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  onImport: (signed: { raw: string; hash: string }) => void;
}) {
  const rows = reviewRows(preview);
  const [qr, setQr] = useState<string | null>(null);
  const [payload, setPayload] = useState<string | null>(null);
  const [incoming, setIncoming] = useState("");
  const unsigned = JSON.stringify(exportUnsigned(preview));

  return (
    <div className="fixed inset-0 z-30 flex items-end justify-center bg-ink/80 p-4 sm:items-center" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <div className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-card border border-line bg-surface p-5 shadow-2xl">
        <p className="text-xs tracking-[0.18em] text-copper uppercase">Review before signing</p>
        <h2 id="confirm-title" className="mt-2 text-2xl font-semibold text-paper">
          Confirm transaction
        </h2>
        <p className="mt-2 text-sm text-muted">Nothing is signed until you confirm. The chain id below is the one the node reported.</p>
        <dl className="mt-5 space-y-3">
          {rows.map((row) => (
            <div key={row.label} className="grid gap-1 border-b border-line pb-3">
              <dt className="text-xs tracking-wide text-muted uppercase">{row.label}</dt>
              <dd className={row.mono ? "mono text-sm break-all text-paper" : "text-sm text-paper"}>{row.value}</dd>
            </div>
          ))}
        </dl>
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
          {payload ? (
            <div className="mt-3 grid gap-3">
              {qr ? <img src={qr} alt="Unsigned transaction QR" className="h-56 w-56 rounded-2xl bg-paper p-2" /> : <p className="text-sm text-muted">The payload is below if it does not fit in a QR.</p>}
              <textarea className="min-h-24 rounded-2xl border border-line bg-ink p-3 font-mono text-xs text-paper" readOnly value={payload} />
              <button type="button" className="min-h-11 w-fit text-sm text-copper" onClick={() => { void navigator.clipboard.writeText(payload); toast("Unsigned payload copied"); }}>
                Copy unsigned JSON
              </button>
            </div>
          ) : null}
          <label className="mt-4 grid gap-1 text-sm text-muted">
            Signed payload from the phone
            <textarea className="min-h-24 rounded-2xl border border-line bg-ink p-3 font-mono text-xs text-paper" value={incoming} onChange={(e) => setIncoming(e.target.value)} placeholder='{"v":1,"role":"signed",...}' />
          </label>
          <button
            type="button"
            className="mt-3 min-h-11 rounded-full border border-copper px-4 text-sm text-copper disabled:opacity-40"
            disabled={busy || !incoming.trim()}
            onClick={() => {
              try {
                const signed = acceptSigned(incoming.trim());
                onImport({ raw: signed.raw, hash: signed.hash });
              } catch (error) {
                toast(error instanceof Error ? error.message : "That payload was rejected");
              }
            }}
          >
            Import signed transaction
          </button>
        </div>
      </div>
    </div>
  );
}