import { create } from "zustand";
import type { PublicAccount, VaultFile, VaultKind } from "@/core/keystore";

export type SessionStatus = "empty" | "locked" | "unlocked";

type SessionState = {
  status: SessionStatus;
  kind: VaultKind | null;
  wordCount: 12 | 24 | null;
  usesBip39Passphrase: boolean;
  accounts: PublicAccount[];
  active: number;
  busy: string | null;
  error: string | null;
  setBusy: (busy: string | null) => void;
  setError: (error: string | null) => void;
  hydrate: (file: VaultFile | null) => void;
  setUnlocked: (accounts: PublicAccount[], active: number) => void;
  setAccounts: (accounts: PublicAccount[], active: number) => void;
  openSession: (input: { kind: VaultKind; wordCount: 12 | 24 | null; usesBip39Passphrase: boolean; accounts: PublicAccount[]; active: number }) => void;
  showLocked: (input: { kind: VaultKind; wordCount: 12 | 24 | null; usesBip39Passphrase: boolean; accounts: PublicAccount[] }) => void;
  lock: () => void;
};

export const useSession = create<SessionState>((set) => ({
  status: "empty",
  kind: null,
  wordCount: null,
  usesBip39Passphrase: false,
  accounts: [],
  active: 0,
  busy: null,
  error: null,
  setBusy: (busy) => set({ busy }),
  setError: (error) => set({ error }),
  hydrate: (file) =>
    set(
      file
        ? {
            status: "locked",
            kind: file.kind,
            wordCount: file.wordCount,
            usesBip39Passphrase: file.usesBip39Passphrase,
            accounts: file.accounts,
            active: 0,
            error: null,
          }
        : {
            status: "empty",
            kind: null,
            wordCount: null,
            usesBip39Passphrase: false,
            accounts: [],
            active: 0,
          },
    ),
  setUnlocked: (accounts, active) => set({ status: "unlocked", accounts, active, error: null }),
  setAccounts: (accounts, active) => set({ accounts, active }),
  openSession: (input) => set({ status: "unlocked", error: null, busy: null, ...input }),
  showLocked: (input) => set({ status: "locked", error: null, busy: null, active: 0, ...input }),
  lock: () => set({ status: "locked", error: null, busy: null }),
}));
