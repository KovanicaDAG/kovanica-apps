import { decodeAddress } from "@/core/address";
import { entropyFromPhrase, isValidMnemonic, mnemonicWordCount, normalizeMnemonic } from "@/core/hd";
import { openVault, sealVault, type PublicAccount, type VaultFile, type VaultKind } from "@/core/keystore";
import {
  derivePublicAccount,
  describeAccount,
  isUnlocked,
  lockSession,
  previewCall,
  previewTransfer,
  revealPhrase,
  signPreview,
  unlockSession,
  type PreviewDetail,
  type TxPreview,
} from "@/core/session";
import { signingHash } from "@/core/tx";
import { bytesToHex, wipe } from "@/core/bytes";
import { decodeRecipient } from "@/core/address";
import { useSession } from "@/stores/session";
import { isDesktopShell, tauriInvoke } from "@/services/tauri";

export { isDesktopShell };

const KEY = "kvnc.keystore.v2";
const DESKTOP_KEY = "kvnc.desktop.meta.v1";

type DesktopMeta = {
  kind: VaultKind;
  wordCount: 12 | 24 | null;
  usesBip39Passphrase: boolean;
  accounts: PublicAccount[];
};

let bip39Memory = "";
let backupMnemonic: string | null = null;
const backupListeners = new Set<(value: string | null) => void>();

export function subscribeBackup(listener: (value: string | null) => void): () => void {
  backupListeners.add(listener);
  listener(backupMnemonic);
  return () => backupListeners.delete(listener);
}

function setBackup(value: string | null) {
  backupMnemonic = value;
  for (const listener of backupListeners) listener(value);
}

export function clearBackup() {
  setBackup(null);
}

function loadDesktopMeta(): DesktopMeta | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const parsed = JSON.parse(localStorage.getItem(DESKTOP_KEY) ?? "null") as DesktopMeta | null;
    if (!parsed || !Array.isArray(parsed.accounts)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function saveDesktopMeta(meta: DesktopMeta) {
  localStorage.setItem(DESKTOP_KEY, JSON.stringify(meta));
}

function publicAccount(kind: VaultKind, accountIndex: number, address: string): PublicAccount {
  return describeAccount(kind, { account: accountIndex, change: 0, index: 0 }, decodeAddress(address));
}

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

export async function createDesktopWallet(words: 12 | 24, password: string): Promise<void> {
  if (!isDesktopShell()) throw new Error("desktop signer is not available");
  requirePassword(password);
  const created = await tauriInvoke<{ address: string; mnemonic: string }>("wallet_create_hd", {
    wordCount: words,
    passphrase: password,
  });
  bip39Memory = "";
  const meta: DesktopMeta = {
    kind: "hd",
    wordCount: words,
    usesBip39Passphrase: false,
    accounts: [publicAccount("hd", 0, created.address)],
  };
  saveDesktopMeta(meta);
  useSession.getState().openSession({ ...meta, active: 0 });
  setBackup(created.mnemonic);
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
  if (isDesktopShell()) {
    const command = input.mode === "legacy" ? "wallet_import" : "wallet_import_hd";
    const imported = await tauriInvoke<{ phase: string; address: string }>(command, {
      mnemonic: phrase,
      passphrase: input.password,
    });
    const bip39 = input.mode === "legacy" ? "" : input.bip39Passphrase;
    bip39Memory = bip39;
    let address = imported.address;
    if (input.mode === "hd" && bip39) {
      address = await tauriInvoke<string>("wallet_derive_account", { bip39Passphrase: bip39, accountIndex: 0 });
    }
    const kind: VaultKind = input.mode === "legacy" ? "legacy" : "hd";
    const meta: DesktopMeta = {
      kind,
      wordCount: count,
      usesBip39Passphrase: bip39.length > 0,
      accounts: [publicAccount(kind, 0, address)],
    };
    saveDesktopMeta(meta);
    useSession.getState().openSession({ ...meta, active: 0 });
    return;
  }
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
  if (isDesktopShell()) {
    const meta = loadDesktopMeta();
    if (!meta) throw new Error("no wallet on this device");
    await tauriInvoke("wallet_unlock", { passphrase: password });
    bip39Memory = meta.usesBip39Passphrase ? bip39Passphrase : "";
    const accounts = meta.kind === "hd"
      ? await Promise.all(meta.accounts.map(async (account) => publicAccount("hd", account.account, await tauriInvoke<string>("wallet_derive_account", {
          bip39Passphrase: bip39Memory,
          accountIndex: account.account,
        }))))
      : meta.accounts;
    const next = { ...meta, accounts };
    saveDesktopMeta(next);
    useSession.getState().openSession({ ...next, active: Math.min(useSession.getState().active, Math.max(accounts.length - 1, 0)) });
    return;
  }
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

export function lockUi(): void {
  setBackup(null);
  bip39Memory = "";
  lockSession();
  useSession.getState().lock();
}

export async function lock(): Promise<void> {
  if (isDesktopShell()) await tauriInvoke("wallet_lock").catch(() => undefined);
  lockUi();
}

export function activeAccount(): PublicAccount | null {
  const { accounts, active, status } = useSession.getState();
  if (status !== "unlocked") return null;
  return accounts[active] ?? null;
}

export async function addAccount(): Promise<void> {
  const state = useSession.getState();
  if (isDesktopShell()) {
    const meta = loadDesktopMeta();
    if (state.status !== "unlocked" || !meta || meta.kind !== "hd") throw new Error("only an unlocked HD wallet can add an account");
    const nextIndex = meta.accounts.reduce((max, account) => Math.max(max, account.account), -1) + 1;
    const address = await tauriInvoke<string>("wallet_derive_account", {
      bip39Passphrase: bip39Memory,
      accountIndex: nextIndex,
    });
    const accounts = [...meta.accounts, publicAccount("hd", nextIndex, address)];
    saveDesktopMeta({ ...meta, accounts });
    useSession.getState().setAccounts(accounts, accounts.length - 1);
    return;
  }
  const file = loadVault();
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
  details: PreviewDetail[];
}): TxPreview {
  const from = activeAccount();
  if (!from) throw new Error("unlock the wallet before signing");
  return previewCall({ from, ...input });
}

export async function signReviewed(preview: TxPreview): Promise<{ raw: string; hash: string }> {
  const from = activeAccount();
  if (!from) throw new Error("wallet is locked");
  if (isDesktopShell()) {
    if (preview.kind !== "transfer") throw new Error("The desktop signer currently signs transfers. Contract calls stay in the preview signer.");
    const raw = await tauriInvoke<string>("wallet_sign_transfer_hd", {
      bip39Passphrase: bip39Memory,
      accountIndex: from.account,
      toAddress: preview.counterparty,
      amount: preview.amountAtoms,
      fee: preview.feeAtoms,
      nonce: Number(preview.nonce),
      chainId: preview.chainId,
    });
    const hash = signingHash(
      {
        sender: decodeAddress(preview.sender),
        nonce: BigInt(preview.nonce),
        fee: BigInt(preview.feeAtoms),
        kind: { type: "transfer", to: decodeRecipient(preview.counterparty, preview.chainId), amount: BigInt(preview.amountAtoms) },
      },
      preview.chainId,
    );
    return { raw: raw.replace(/^0x/, ""), hash: bytesToHex(hash) };
  }
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

export async function refreshDesktopSession(): Promise<void> {
  if (!isDesktopShell()) return;
  const meta = loadDesktopMeta();
  try {
    const status = await tauriInvoke<Record<string, unknown>>("wallet_status");
    const unlocked = status.unlocked === true || status.phase === "unlocked";
    const address = typeof status.address === "string" ? status.address : meta?.accounts[0]?.address;
    if (!meta && address) {
      const kind: VaultKind = status.kind === "legacy" || status.legacy === true ? "legacy" : "hd";
      const created: DesktopMeta = {
        kind,
        wordCount: kind === "legacy" ? 24 : null,
        usesBip39Passphrase: false,
        accounts: [publicAccount(kind, 0, address)],
      };
      saveDesktopMeta(created);
      if (unlocked) useSession.getState().openSession({ ...created, active: 0 });
      else useSession.getState().showLocked(created);
      return;
    }
    if (meta && unlocked) useSession.getState().openSession({ ...meta, active: 0 });
    else if (meta) useSession.getState().showLocked(meta);
  } catch {
    if (meta) useSession.getState().showLocked(meta);
  }
}

export function forgetDevice() {
  clearBackup();
  bip39Memory = "";
  lockSession();
  if (isDesktopShell()) {
    localStorage.removeItem(DESKTOP_KEY);
    void tauriInvoke("wallet_lock").catch(() => undefined);
  }
  localStorage.removeItem(KEY);
  useSession.getState().hydrate(null);
}
