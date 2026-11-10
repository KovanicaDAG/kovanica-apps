import { create } from "zustand";

export type TxStatus = "pending" | "confirmed" | "failed";
export type TxDirection = "sent" | "received" | "contract";

export type ActivityRow = {
  hash: string;
  status: TxStatus;
  direction: TxDirection;
  amountAtoms: string;
  feeAtoms: string;
  counterparty: string;
  chainId: number;
  createdAt: number;
  height: number | null;
  note: string;
  gasUsed?: string | null;
  code?: string | null;
  error?: string | null;
  logs?: number | null;
  events?: string[] | null;
};

const KEY = "kvnc.activity.v1";

function load(): ActivityRow[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const parsed = JSON.parse(localStorage.getItem(KEY) ?? "[]") as ActivityRow[];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

type ActivityState = {
  rows: ActivityRow[];
  indexerNote: string;
  hydrate: () => void;
  push: (row: ActivityRow) => void;
  mark: (hash: string, status: TxStatus, height: number | null, extra?: { gasUsed?: string | null; code?: string | null; error?: string | null; logs?: number | null; events?: string[] | null }) => void;
  mergeIncoming: (rows: ActivityRow[]) => void;
  setIndexerNote: (indexerNote: string) => void;
};

function save(rows: ActivityRow[]) {
  localStorage.setItem(KEY, JSON.stringify(rows.slice(0, 80)));
}

export const useActivity = create<ActivityState>((set, get) => ({
  rows: [],
  indexerNote: "Outgoing transactions only",
  hydrate: () => set({ rows: load() }),
  push: (row) => {
    const rows = [row, ...get().rows.filter((item) => item.hash !== row.hash)];
    save(rows);
    set({ rows });
  },
  mark: (hash, status, height, extra) => {
    const rows = get().rows.map((row) => (row.hash === hash ? { ...row, status, height, ...extra } : row));
    save(rows);
    set({ rows });
  },
  mergeIncoming: (incoming) => {
    const known = new Set(get().rows.map((row) => row.hash));
    const rows = [...incoming.filter((row) => !known.has(row.hash)), ...get().rows];
    save(rows);
    set({ rows });
  },
  setIndexerNote: (indexerNote) => set({ indexerNote }),
}));
