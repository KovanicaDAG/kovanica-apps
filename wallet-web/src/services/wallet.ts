import { entropyFromPhrase, isValidMnemonic, mnemonicWordCount, normalizeMnemonic } from "@/core/hd";
import { openVault, sealVault, type PublicAccount, type VaultFile } from "@/core/keystore";
import {
  derivePublicAccount,
  isUnlocked,
  lockSession,
  previewCall,
  previewTransfer,
  revealPhrase,
  signPreview,
  unlockSession,
  type TxPreview,
} from "@/core/session";
import { wipe } from "@/core/bytes";
import { useSession } from "@/stores/session";

const KEY = "kvnc.keystore.v2";

export function loadVault(): VaultFile | null {
  if (typeof localStorage === "undefined") return null;
  const raw = localStorage.getItem(KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as VaultFile;
    if (parsed.version !== 2 || !Array.isArray(parsed.accounts)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveVault(file: VaultFile) {
  localStorage.setItem(KEY, JSON.stringify(file));
  useSession.getState().hydrate(file);
}

export function hasVault(): boolean {
  return loadVault() != null;
}

function requirePassword(password: string) {
  if (password.length < 8) throw new Error("password must be at least 8 characters");
}

export async function createHdWallet(phrase: string, password: string): Promise<void> {
  requirePassword(password);
  const normalized = normalizeMnemonic(phrase);
  if (!isValidMnemonic(normalized)) throw new Error("checksum does not match this word list");
  const count = mnemonicWordCount(normalized);
  if (count !== 12 && count !== 24) throw new Error("use 12 or 24 words");
  const entropy = entropyFromPhrase(normalized);
  unlockSession({ kind: "hd", entropy, wordCount: count }, "");
  const account = derivePublicAccount({ account: 0, change: 0, index: 0 });
  const file = await sealVault(
    { kind: "hd", entropy: entropy.slice(), wordCount: count },
    password,
    { kind: "hd", wordCount: count, usesBip39Passphrase: false, accounts: [account] },
  );
  wipe(entropy);
  saveVault(file);
  useSession.getState().setUnlocked(file.accounts, 0);
}

export async function importWallet(input: {
  phrase: string;
  mode: "hd" | "legacy";
  password: string;
  bip39Passphrase: string;
}): Promise<void> {
  requirePassword(input.password);
  const phrase = normalizeMnemonic(input.phrase);
  if (!isValidMnemonic(phrase)) throw new Error("checksum does not match this word list");
  const count = mnemonicWordCount(phrase);
  if (count !== 12 && count !== 24) throw new Error("use 12 or 24 words");
  if (input.mode === "legacy" && count !== 24) throw new Error("legacy accounts use a 24-word phrase");
  const entropy = entropyFromPhrase(phrase);
  if (input.mode === "legacy") {
    unlockSession({ kind: "legacy", seed: entropy }, "");
    const account = derivePublicAccount({ account: 0, change: 0, index: 0 });
    const file = await sealVault(
      { kind: "legacy", seed: entropy.slice() },
      input.password,
      { kind: "legacy", wordCount: 24, usesBip39Passphrase: false, accounts: [account] },
    );
    wipe(entropy);
    saveVault(file);
    useSession.getState().setUnlocked(file.accounts, 0);
    return;
  }
  const opened = { kind: "hd" as const, entropy, wordCount: count as 12 | 24 };
  unlockSession(opened, input.bip39Passphrase);
  const account = derivePublicAccount({ account: 0, change: 0, index: 0 });
  const file = await sealVault(
    { kind: "hd", entropy: entropy.slice(), wordCount: count as 12 | 24 },
    input.password,
    {
      kind: "hd",
      wordCount: count as 12 | 24,
      usesBip39Passphrase: input.bip39Passphrase.length > 0,
      accounts: [account],
    },
  );
  wipe(entropy);
  saveVault(file);
  useSession.getState().setUnlocked(file.accounts, 0);
}

export async function unlock(password: string, bip39Passphrase: string): Promise<void> {
  const file = loadVault();
  if (!file) throw new Error("no wallet on this device");
  const opened = await openVault(file, password);
  if (file.kind === "legacy" && opened.kind !== "legacy") throw new Error("keystore kind mismatch");
  if (file.kind === "hd" && opened.kind !== "hd") throw new Error("keystore kind mismatch");
  unlockSession(opened, file.usesBip39Passphrase ? bip39Passphrase : "");
  if (opened.kind === "hd") wipe(opened.entropy);
  else wipe(opened.seed);
  const checked = file.accounts.map((account) =>
    derivePublicAccount({ account: account.account, change: account.change, index: account.index }),
  );
  const next = { ...file, accounts: checked };
  saveVault(next);
  useSession.getState().setUnlocked(checked, useSession.getState().active);
}

export function lock(): void {
  lockSession();
  useSession.getState().lock();
}

export function activeAccount(): PublicAccount | null {
  const { accounts, active, status } = useSession.getState();
  if (status !== "unlocked") return null;
  return accounts[active] ?? null;
}

export async function addAccount(): Promise<void> {
  const file = loadVault();
  const state = useSession.getState();
  if (!file || state.kind !== "hd" || !isUnlocked()) throw new Error("only an unlocked HD wallet can add an account");
  const nextIndex = file.accounts.reduce((max, account) => Math.max(max, account.account), -1) + 1;
  const account = derivePublicAccount({ account: nextIndex, change: 0, index: 0 });
  const accounts = [...file.accounts, account];
  saveVault({ ...file, accounts });
  useSession.getState().setAccounts(accounts, accounts.length - 1);
}

export function selectAccount(index: number) {
  const { accounts } = useSession.getState();
  if (!accounts[index]) return;
  useSession.getState().setAccounts(accounts, index);
}

export function buildTransferPreview(input: {
  recipient: string;
  amountAtoms: bigint;
  feeAtoms: bigint;
  nonce: bigint;
  chainId: number;
  networkName: string;
}): TxPreview {
  const from = activeAccount();
  if (!from) throw new Error("unlock the wallet before signing");
  return previewTransfer({ from, ...input });
}

export function buildCallPreview(input: {
  contract: string;
  method: string;
  args: Uint8Array;
  valueAtoms: bigint;
  gasLimit: bigint;
  feeAtoms: bigint;
  nonce: bigint;
  chainId: number;
  networkName: string;
}): TxPreview {
  const from = activeAccount();
  if (!from) throw new Error("unlock the wallet before signing");
  return previewCall({ from, ...input });
}

export function signReviewed(preview: TxPreview): { raw: string; hash: string } {
  const from = activeAccount();
  if (!from) throw new Error("wallet is locked");
  return signPreview(preview, from);
}

export async function revealWithPassword(password: string): Promise<string> {
  const file = loadVault();
  if (!file) throw new Error("no wallet on this device");
  const opened = await openVault(file, password);
  wipe(opened.kind === "hd" ? opened.entropy : opened.seed);
  if (!isUnlocked()) throw new Error("unlock the wallet first");
  return revealPhrase();
}

export function forgetDevice() {
  lockSession();
  localStorage.removeItem(KEY);
  useSession.getState().hydrate(null);
}
