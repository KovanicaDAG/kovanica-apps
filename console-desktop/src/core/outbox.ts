/** Contract calls this console broadcast, kept in memory for the history list. */

export type OutboxRow = {
  hash: string;
  method: string;
  counterparty: string;
  status: "pending" | "success" | "revert";
  code: "0x1" | "0x0" | null;
  gasUsed: string | null;
  error: string | null;
  events: string[];
};

const rows: OutboxRow[] = [];
const listeners = new Set<() => void>();

export function outboxRows(): OutboxRow[] {
  return rows.slice();
}

export function pushOutbox(row: OutboxRow): void {
  const next = rows.filter((item) => item.hash !== row.hash);
  next.unshift(row);
  rows.splice(0, rows.length, ...next.slice(0, 40));
  listeners.forEach((listener) => listener());
}

export function updateOutbox(hash: string, patch: Partial<OutboxRow>): void {
  const row = rows.find((item) => item.hash === hash);
  if (!row) return;
  Object.assign(row, patch);
  listeners.forEach((listener) => listener());
}

export function subscribeOutbox(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
