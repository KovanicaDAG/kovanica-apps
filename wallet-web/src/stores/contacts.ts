import { checkAddress } from "@/core/address";

export type Contact = { id: string; label: string; address: string };

const KEY = "kvnc.contacts.v1";

export function loadContacts(): Contact[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const parsed = JSON.parse(localStorage.getItem(KEY) ?? "[]") as Contact[];
    return Array.isArray(parsed) ? parsed.filter((item) => item && typeof item.address === "string" && typeof item.label === "string") : [];
  } catch {
    return [];
  }
}

export function saveContact(label: string, address: string, chainId: number | null): Contact {
  const trimmed = label.trim();
  if (!trimmed) throw new Error("Give the contact a label");
  const check = checkAddress(address, chainId);
  if (!check.ok) throw new Error(check.message);
  const contact: Contact = { id: crypto.randomUUID(), label: trimmed, address: address.trim().toLowerCase() };
  const next = [contact, ...loadContacts().filter((item) => item.address !== contact.address)];
  localStorage.setItem(KEY, JSON.stringify(next.slice(0, 80)));
  return contact;
}

export function removeContact(id: string): Contact[] {
  const next = loadContacts().filter((item) => item.id !== id);
  localStorage.setItem(KEY, JSON.stringify(next));
  return next;
}
