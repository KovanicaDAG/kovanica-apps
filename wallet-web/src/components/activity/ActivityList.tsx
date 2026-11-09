import { formatKvnc } from "@/core/format";
import { useActivity } from "@/stores/activity";

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
            <p className="text-xs tracking-wide text-copper uppercase">{row.status}</p>
          </div>
          <p className="mt-2 tabular-nums text-paper">{formatKvnc(BigInt(row.amountAtoms))} KVNC</p>
          <p className="mono mt-2 text-xs break-all text-muted">{row.hash}</p>
          <a className="mt-2 inline-block text-sm text-copper" href={`https://explorer.kovanica.online/tx/${row.hash}`}>
            Explorer
          </a>
        </article>
      ))}
    </div>
  );
}
