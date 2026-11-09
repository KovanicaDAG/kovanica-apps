import type { TxPreview } from "@/core/session";
import { reviewRows } from "@/services/tx";

export function ConfirmTx({
  preview,
  busy,
  onCancel,
  onConfirm,
}: {
  preview: TxPreview;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const rows = reviewRows(preview);
  return (
    <div className="fixed inset-0 z-30 flex items-end justify-center bg-ink/80 p-4 sm:items-center" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <div className="w-full max-w-lg rounded-card border border-line bg-surface p-5 shadow-2xl">
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
      </div>
    </div>
  );
}
