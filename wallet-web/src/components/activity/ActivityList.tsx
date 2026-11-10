import { formatKvnc } from "@/core/format";
import { useActivity, type ActivityRow } from "@/stores/activity";

function badge(row: ActivityRow): string {
  if (row.status === "confirmed") return row.height == null ? "Confirmed" : `Confirmed (${row.height})`;
  if (row.status === "failed") return "Failed";
  return "Pending";
}

export function ActivityList() {
  const rows = useActivity((s) => s.rows);
  const note = useActivity((s) => s.indexerNote);
  return (
    <div className="grid gap-3">
      <p className="text-sm text-muted">{note}</p>
      {rows.length === 0 ? <p className="rounded-card border border-line bg-ink px-4 py-6 text-sm text-muted">No transactions from this device yet.</p> : null}
      {rows.map((row) => (
        <article key={row.hash} className="rounded-card border border-line bg-ink px-4 py-3">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm capitalize text-paper">{row.direction}</p>
            <p className={`rounded-full border px-2 py-1 text-xs tracking-wide uppercase ${row.status === "pending" ? "border-copper text-copper" : row.status === "failed" ? "border-clay text-clay" : "border-sage text-sage"}`}>{badge(row)}</p>
          </div>
          <p className="mt-2 tabular-nums text-paper">{formatKvnc(BigInt(row.amountAtoms))} KVNC</p>
          <p className="mt-1 text-sm text-muted">To {row.counterparty || "—"} · fee {formatKvnc(BigInt(row.feeAtoms))} KVNC</p>
          <p className="mt-1 text-xs text-muted">{new Date(row.createdAt).toLocaleString()}</p>
          <p className="mono mt-2 text-xs break-all text-muted">{row.hash}</p>
          <a className="mt-2 inline-block text-sm text-copper" href={`https://explorer.kovanica.online/tx/${row.hash}`}>
            Explorer
          </a>
        </article>
      ))}
    </div>
  );
}