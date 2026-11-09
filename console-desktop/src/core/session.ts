import { encodeAddress } from "./address";
import { decodeRecipient } from "./address";
import { bytesToHex, wipe } from "./bytes";
import { formatBip44, hdPrivateKey, phraseFromEntropy } from "./hd";
import type { OpenedVault, PublicAccount, VaultKind } from "./keystore";
import { publicKeyFromSeed, signTransaction, type UnsignedTx } from "./tx";

export type AccountRef = { account: number; change: number; index: number };

type LiveSecret =
  | { kind: "hd"; entropy: Uint8Array; wordCount: 12 | 24; bip39Passphrase: string }
  | { kind: "legacy"; seed: Uint8Array };

let live: LiveSecret | null = null;

export function isUnlocked(): boolean {
  return live !== null;
}

export function lockSession(): void {
  if (!live) return;
  if (live.kind === "hd") {
    wipe(live.entropy);
    live.bip39Passphrase = "";
  } else {
    wipe(live.seed);
  }
  live = null;
}

export function unlockSession(opened: OpenedVault, bip39Passphrase: string): void {
  lockSession();
  if (opened.kind === "hd") {
    live = {
      kind: "hd",
      entropy: opened.entropy.slice(),
      wordCount: opened.wordCount,
      bip39Passphrase,
    };
    return;
  }
  live = { kind: "legacy", seed: opened.seed.slice() };
}

function childSeed(ref: AccountRef): Uint8Array {
  if (!live) throw new Error("wallet is locked");
  if (live.kind === "legacy") {
    if (ref.account !== 0 || ref.change !== 0 || ref.index !== 0) {
      throw new Error("a legacy account is a single key, not an HD path");
    }
    return live.seed.slice();
  }
  return hdPrivateKey(live.entropy, live.bip39Passphrase, ref.account, ref.change, ref.index);
}

export function describeAccount(kind: VaultKind, ref: AccountRef, publicKey: Uint8Array): PublicAccount {
  return {
    kind,
    account: ref.account,
    change: ref.change,
    index: ref.index,
    address: encodeAddress(publicKey),
    publicKey: bytesToHex(publicKey),
    pathLabel: kind === "legacy" ? "Legacy account" : formatBip44(ref.account, ref.change, ref.index),
  };
}

export function derivePublicAccount(ref: AccountRef): PublicAccount {
  const seed = childSeed(ref);
  try {
    const publicKey = publicKeyFromSeed(seed);
    const kind = live?.kind === "legacy" ? "legacy" : "hd";
    return describeAccount(kind, ref, publicKey);
  } finally {
    wipe(seed);
  }
}

export type PreviewDetail = { label: string; value: string; mono?: boolean };

export type TxPreview = {
  kind: "transfer" | "call";
  sender: string;
  counterparty: string;
  amountAtoms: string;
  feeAtoms: string;
  nonce: string;
  chainId: number;
  networkName: string;
  method: string;
  argsHex: string;
  gasLimit: string;
  valueAtoms: string;
  pathLabel: string;
  details: PreviewDetail[];
};

function assertU64(value: bigint, label: string): void {
  if (value < 0n || value > 0xffffffffffffffffn) throw new Error(`${label} is out of range`);
}

export function previewTransfer(input: {
  from: PublicAccount;
  recipient: string;
  amountAtoms: bigint;
  feeAtoms: bigint;
  nonce: bigint;
  chainId: number;
  networkName: string;
}): TxPreview {
  decodeRecipient(input.recipient, input.chainId);
  assertU64(input.amountAtoms, "amount");
  assertU64(input.feeAtoms, "fee");
  assertU64(input.nonce, "nonce");
  if (input.amountAtoms === 0n) throw new Error("amount must be greater than zero");
  return {
    kind: "transfer",
    sender: input.from.address,
    counterparty: input.recipient.trim().toLowerCase(),
    amountAtoms: input.amountAtoms.toString(),
    feeAtoms: input.feeAtoms.toString(),
    nonce: input.nonce.toString(),
    chainId: input.chainId,
    networkName: input.networkName,
    method: "",
    argsHex: "",
    gasLimit: "0",
    valueAtoms: "0",
    pathLabel: input.from.pathLabel,
    details: [],
  };
}

export function previewCall(input: {
  from: PublicAccount;
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
  decodeRecipient(input.contract, input.chainId);
  if (!input.method.trim()) throw new Error("method must not be empty");
  assertU64(input.valueAtoms, "value");
  assertU64(input.gasLimit, "gas");
  assertU64(input.feeAtoms, "fee");
  assertU64(input.nonce, "nonce");
  return {
    kind: "call",
    sender: input.from.address,
    counterparty: input.contract.trim().toLowerCase(),
    amountAtoms: input.valueAtoms.toString(),
    feeAtoms: input.feeAtoms.toString(),
    nonce: input.nonce.toString(),
    chainId: input.chainId,
    networkName: input.networkName,
    method: input.method.trim(),
    argsHex: bytesToHex(input.args),
    gasLimit: input.gasLimit.toString(),
    valueAtoms: input.valueAtoms.toString(),
    pathLabel: input.from.pathLabel,
    details: input.details,
  };
}

function unsignedFromPreview(preview: TxPreview, senderKey: Uint8Array): UnsignedTx {
  const sender = publicKeyFromSeed(senderKey);
  if (encodeAddress(sender) !== preview.sender) {
    wipe(sender);
    throw new Error("preview sender does not match the unlocked account");
  }
  const base = {
    sender,
    nonce: BigInt(preview.nonce),
    fee: BigInt(preview.feeAtoms),
  };
  if (preview.kind === "transfer") {
    return {
      ...base,
      kind: {
        type: "transfer",
        to: decodeRecipient(preview.counterparty, preview.chainId),
        amount: BigInt(preview.amountAtoms),
      },
    };
  }
  return {
    ...base,
    kind: {
      type: "call",
      contract: decodeRecipient(preview.counterparty, preview.chainId),
      value: BigInt(preview.valueAtoms),
      method: preview.method,
      args: preview.argsHex ? hexToBytesSafe(preview.argsHex) : new Uint8Array(),
      gasLimit: BigInt(preview.gasLimit),
    },
  };
}

function hexToBytesSafe(hex: string): Uint8Array {
  if (hex.length % 2 !== 0 || /[^0-9a-f]/i.test(hex)) throw new Error("call args must be hex");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function signPreview(preview: TxPreview, from: PublicAccount): { raw: string; hash: string } {
  if (from.address !== preview.sender || from.pathLabel !== preview.pathLabel) {
    throw new Error("preview does not match the selected account");
  }
  const seed = childSeed(from);
  try {
    const unsigned = unsignedFromPreview(preview, seed);
    const signed = signTransaction(unsigned, preview.chainId, seed);
    wipe(unsigned.sender);
    wipe(signed.signature);
    return { raw: signed.raw, hash: signed.hash };
  } finally {
    wipe(seed);
  }
}

export function revealPhrase(): string {
  if (!live) throw new Error("wallet is locked");
  if (live.kind === "legacy") return phraseFromEntropy(live.seed);
  return phraseFromEntropy(live.entropy);
}
