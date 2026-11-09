import { checkAddress } from "@/core/address";

export type WatchKind = "token" | "vault" | "htlc" | "multisig";

export type WatchItem = {
  id: string;
  kind: WatchKind;
  label: string;
  address: string;
};

const KEY = "kvnc.watch.v1";

export function loadWatch(): WatchItem[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const parsed = JSON.parse(localStorage.getItem(KEY) ?? "[]") as WatchItem[];
    return Array.isArray(parsed) ? parsed.filter((item) => item && typeof item.address === "string" && typeof item.kind === "string") : [];
  } catch {
    return [];
  }
}

export function saveWatch(kind: WatchKind, label: string, address: string, chainId: number | null): WatchItem {
  const trimmed = label.trim();
  if (!trimmed) throw new Error("Give this contract a label");
  const check = checkAddress(address, chainId);
  if (!check.ok) throw new Error(check.message);
  const item: WatchItem = { id: crypto.randomUUID(), kind, label: trimmed, address: address.trim().toLowerCase() };
  const next = [item, ...loadWatch().filter((row) => !(row.address === item.address && row.kind === item.kind))];
  localStorage.setItem(KEY, JSON.stringify(next.slice(0, 40)));
  return item;
}

export function removeWatch(id: string): WatchItem[] {
  const next = loadWatch().filter((item) => item.id !== id);
  localStorage.setItem(KEY, JSON.stringify(next));
  return next;
}

export function replaceWatch(items: WatchItem[]) {
  localStorage.setItem(KEY, JSON.stringify(items.slice(0, 40)));
}
