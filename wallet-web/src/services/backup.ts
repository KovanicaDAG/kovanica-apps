import { loadContacts } from "@/stores/contacts";
import { loadWatch, replaceWatch, type WatchItem } from "@/stores/watch";
import { loadVault } from "@/services/wallet";
import { lockSession } from "@/core/session";
import { useSession } from "@/stores/session";
import type { VaultFile } from "@/core/keystore";

const CONTACT_KEY = "kvnc.contacts.v1";

export function backupJson(): string {
  return JSON.stringify({
    version: 1,
    keystore: loadVault(),
    contacts: loadContacts(),
    watch: loadWatch(),
  });
}

export function restoreBackup(text: string): void {
  const parsed = JSON.parse(text) as { version?: number; keystore?: VaultFile | null; contacts?: unknown; watch?: WatchItem[] };
  if (parsed.version !== 1) throw new Error("This backup is not a Kovanica wallet file");
  if (parsed.keystore) {
    if (parsed.keystore.version !== 2 || !Array.isArray(parsed.keystore.accounts)) throw new Error("The keystore in this file is not version 2");
    localStorage.setItem("kvnc.keystore.v2", JSON.stringify(parsed.keystore));
    lockSession();
    useSession.getState().hydrate(parsed.keystore);
  }
  if (Array.isArray(parsed.contacts)) localStorage.setItem(CONTACT_KEY, JSON.stringify(parsed.contacts));
  if (Array.isArray(parsed.watch)) replaceWatch(parsed.watch);
}
