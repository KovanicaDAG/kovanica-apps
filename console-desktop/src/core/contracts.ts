import { blake3 } from "@noble/hashes/blake3.js";
import { decodeRecipient } from "./address";
import { bytesToHex, concat, hexToBytes, u32le, u64le, u128le } from "./bytes";
import type { PreviewDetail } from "./session";

/** Storage slot for `blake3("kvnc/v1/state")`, the key `kovanica_getStorageAt` expects. */
export const TOKEN_STATE_LABEL = "kvnc/v1/state";

export type CallDraft = {
  method: string;
  args: Uint8Array;
  details: PreviewDetail[];
};

export type ContractName = "htlc" | "vault" | "multisig" | "token";

export type CallInput = {
  contract: ContractName;
  method: string;
  chainId: number | null;
  claimer: string;
  beneficiary: string;
  recipient: string;
  account: string;
  hash: string;
  secret: string;
  id: string;
  timelockMs: bigint | null;
  lockUntilMs: bigint | null;
  amountAtoms: bigint | null;
  tokenUnits: bigint | null;
  txId: bigint | null;
  callDataHex: string;
};

export type TokenBalanceRow = { account: Uint8Array; amount: bigint };

export type TokenState = {
  name: string;
  symbol: string;
  decimals: number;
  totalSupply: bigint;
  paused: boolean;
  icon: string;
  balances: TokenBalanceRow[];
};

function key(address: string, chainId: number | null, label: string): Uint8Array {
  const bytes = decodeRecipient(address, chainId);
  if (bytes.length !== 32) throw new Error(`${label} must be 32 bytes`);
  return bytes;
}

function hex32(value: string, label: string): Uint8Array {
  const clean = value.trim().toLowerCase().replace(/^0x/, "");
  if (!/^[0-9a-f]{64}$/.test(clean)) throw new Error(`${label} must be 32 bytes of hex`);
  return hexToBytes(clean);
}

function millis(value: bigint | null, label: string): Uint8Array {
  if (value == null) throw new Error(`Enter ${label}`);
  return u64le(value);
}

/** Hex of even length is raw bytes. Anything else is the UTF-8 secret. */
export function secretBytes(value: string): Uint8Array {
  const trimmed = value.trim();
  if (!trimmed) throw new Error("Enter the secret");
  const hex = trimmed.toLowerCase().replace(/^0x/, "");
  if (/^[0-9a-f]+$/.test(hex) && hex.length % 2 === 0) return hexToBytes(hex);
  return new TextEncoder().encode(trimmed);
}

function bytesVec(bytes: Uint8Array): Uint8Array {
  return concat([u64le(BigInt(bytes.length)), bytes]);
}

function callData(hex: string): Uint8Array {
  const trimmed = hex.trim().toLowerCase().replace(/^0x/, "");
  if (!trimmed) return new Uint8Array();
  if (!/^[0-9a-f]+$/.test(trimmed) || trimmed.length % 2 !== 0) throw new Error("Call data must be hex");
  return hexToBytes(trimmed);
}

/**
 * Bincode 1.x arguments for the native/wasm entry points in `docs/CONTRACTS.md`.
 * Amounts are `u128`. An absolute vault schedule is variant `0` plus `unlock_at`.
 * HTLC and vault pull `amount` from the caller balance; that is not `Call.value`.
 */
export function buildContractCall(input: CallInput): CallDraft {
  const method = input.method.trim();
  if (input.contract === "htlc" && method === "htlc_create") {
    const claimer = key(input.claimer, input.chainId, "claimer");
    const hash = hex32(input.hash, "hash");
    const atoms = input.amountAtoms;
    if (atoms == null) throw new Error("Enter an amount");
    return {
      method,
      args: concat([claimer, u128le(atoms), hash, millis(input.timelockMs, "a timelock in milliseconds")]),
      details: [
        { label: "Claimer", value: input.claimer.trim().toLowerCase(), mono: true },
        { label: "Hash", value: bytesToHex(hash), mono: true },
        { label: "Timelock (ms)", value: input.timelockMs!.toString() },
        { label: "Amount", value: `${atoms.toString()} atoms` },
        { label: "Balance pull", value: "Contract moves this amount from your balance. Attached value stays separate." },
      ],
    };
  }
  if (input.contract === "htlc" && method === "htlc_claim") {
    const id = hex32(input.id, "swap id");
    const secret = secretBytes(input.secret);
    return {
      method,
      args: concat([id, bytesVec(secret)]),
      details: [
        { label: "Swap id", value: bytesToHex(id), mono: true },
        { label: "Secret", value: bytesToHex(secret), mono: true },
      ],
    };
  }
  if (input.contract === "htlc" && method === "htlc_refund") {
    const id = hex32(input.id, "swap id");
    return {
      method,
      args: id,
      details: [{ label: "Swap id", value: bytesToHex(id), mono: true }],
    };
  }
  if (input.contract === "vault" && method === "vault_create") {
    const beneficiary = key(input.beneficiary, input.chainId, "beneficiary");
    const atoms = input.amountAtoms;
    if (atoms == null) throw new Error("Enter an amount");
    const unlock = input.lockUntilMs;
    if (unlock == null) throw new Error("Enter lock_until_ms");
    return {
      method,
      args: concat([beneficiary, u128le(atoms), u32le(0), u64le(unlock)]),
      details: [
        { label: "Beneficiary", value: input.beneficiary.trim().toLowerCase(), mono: true },
        { label: "Lock until (ms)", value: unlock.toString() },
        { label: "Amount", value: `${atoms.toString()} atoms` },
        { label: "Schedule", value: "Absolute" },
        { label: "Balance pull", value: "Contract moves this amount from your balance. Attached value stays separate." },
      ],
    };
  }
  if (input.contract === "vault" && (method === "vault_claim" || method === "vault_cancel")) {
    const id = hex32(input.id, "vault id");
    return {
      method,
      args: id,
      details: [{ label: method === "vault_claim" ? "Vault id" : "Vault id", value: bytesToHex(id), mono: true }],
    };
  }
  if (input.contract === "multisig" && method === "multisig_propose") {
    const id = hex32(input.id, "multisig id");
    const recipient = key(input.recipient, input.chainId, "recipient");
    const atoms = input.amountAtoms;
    if (atoms == null) throw new Error("Enter an amount");
    const data = callData(input.callDataHex);
    return {
      method,
      args: concat([id, recipient, u128le(atoms), bytesVec(data)]),
      details: [
        { label: "Multisig id", value: bytesToHex(id), mono: true },
        { label: "Recipient", value: input.recipient.trim().toLowerCase(), mono: true },
        { label: "Amount", value: `${atoms.toString()} atoms` },
        { label: "Call data", value: data.length ? bytesToHex(data) : "none", mono: data.length > 0 },
      ],
    };
  }
  if (input.contract === "multisig" && (method === "multisig_confirm" || method === "multisig_execute")) {
    const id = hex32(input.id, "multisig id");
    if (input.txId == null) throw new Error("Enter a proposal id");
    return {
      method,
      args: concat([id, u64le(input.txId)]),
      details: [
        { label: "Multisig id", value: bytesToHex(id), mono: true },
        { label: "Proposal id", value: input.txId.toString() },
      ],
    };
  }
  if (input.contract === "token" && method === "token_transfer") {
    if (input.tokenUnits == null) throw new Error("Enter a token amount");
    const recipient = key(input.recipient, input.chainId, "recipient");
    return {
      method,
      args: concat([recipient, u128le(input.tokenUnits)]),
      details: [
        { label: "Recipient", value: input.recipient.trim().toLowerCase(), mono: true },
        { label: "Token amount", value: input.tokenUnits.toString() },
      ],
    };
  }
  if (input.contract === "token" && method === "token_mint") {
    if (input.tokenUnits == null) throw new Error("Enter a token amount");
    const account = key(input.account, input.chainId, "account");
    return {
      method,
      args: concat([account, u128le(input.tokenUnits)]),
      details: [
        { label: "Account", value: input.account.trim().toLowerCase(), mono: true },
        { label: "Token amount", value: input.tokenUnits.toString() },
      ],
    };
  }
  throw new Error("That contract action is not one of the typed forms");
}

export function tokenStateSlot(): string {
  return bytesToHex(blake3(new TextEncoder().encode(TOKEN_STATE_LABEL)));
}

class Reader {
  private offset = 0;
  private bytes: Uint8Array;

  constructor(bytes: Uint8Array) {
    this.bytes = bytes;
  }

  rest(): number {
    return this.bytes.length - this.offset;
  }

  u8(): number {
    if (this.offset >= this.bytes.length) throw new Error("short");
    return this.bytes[this.offset++]!;
  }

  take(size: number): Uint8Array {
    if (size < 0 || this.offset + size > this.bytes.length) throw new Error("short");
    const out = this.bytes.slice(this.offset, this.offset + size);
    this.offset += size;
    return out;
  }

  u32(): number {
    const view = new DataView(this.take(4).buffer);
    return view.getUint32(0, true);
  }

  u64(): bigint {
    const view = new DataView(this.take(8).buffer);
    const lo = BigInt(view.getUint32(0, true));
    const hi = BigInt(view.getUint32(4, true));
    return lo | (hi << 32n);
  }

  u128(): bigint {
    const view = new DataView(this.take(16).buffer);
    let value = 0n;
    for (let word = 0; word < 4; word += 1) {
      value |= BigInt(view.getUint32(word * 4, true)) << BigInt(word * 32);
    }
    return value;
  }

  str(): string {
    const length = this.u64();
    if (length > BigInt(this.rest())) throw new Error("short");
    return new TextDecoder().decode(this.take(Number(length)));
  }
}

function parseToken(bytes: Uint8Array, withIcon: boolean): TokenState | null {
  try {
    const reader = new Reader(bytes);
    const name = reader.str();
    const symbol = reader.str();
    const decimals = reader.u8();
    const totalSupply = reader.u128();
    reader.take(32);
    reader.take(32);
    const balanceCount = reader.u64();
    if (balanceCount > 100_000n) return null;
    const balances: TokenBalanceRow[] = [];
    for (let i = 0n; i < balanceCount; i += 1n) {
      balances.push({ account: reader.take(32), amount: reader.u128() });
    }
    const allowanceCount = reader.u64();
    if (allowanceCount > 100_000n) return null;
    for (let i = 0n; i < allowanceCount; i += 1n) {
      reader.take(32);
      reader.take(32);
      reader.u128();
    }
    const paused = reader.u8() === 1;
    const icon = withIcon ? reader.str() : "";
    if (reader.rest() !== 0) return null;
    return { name, symbol, decimals, totalSupply, paused, icon, balances };
  } catch {
    return null;
  }
}

/** Current token layout, then the pre-icon layout. */
export function decodeTokenState(bytes: Uint8Array): TokenState | null {
  return parseToken(bytes, true) ?? parseToken(bytes, false);
}

export function storageBytes(value: unknown): Uint8Array | null {
  if (typeof value === "string") {
    const hex = value.trim().toLowerCase().replace(/^0x/, "");
    if (!hex) return new Uint8Array();
    if (!/^[0-9a-f]+$/.test(hex) || hex.length % 2 !== 0) return null;
    return hexToBytes(hex);
  }
  if (Array.isArray(value) && value.every((item) => typeof item === "number" && item >= 0 && item <= 255)) {
    return Uint8Array.from(value);
  }
  return null;
}

/** `blake3(prefix || id)` — the slot `kovanica_getStorageAt` uses for a record. */
export function prefixedSlot(prefix: string, id: Uint8Array): string {
  return bytesToHex(blake3(concat([new TextEncoder().encode(prefix), id])));
}

export type HtlcRecord = {
  sender: string;
  claimer: string;
  amount: bigint;
  hash: string;
  expiry: bigint;
  claimed: boolean;
  refunded: boolean;
};

export function decodeHtlc(bytes: Uint8Array): HtlcRecord | null {
  try {
    const reader = new Reader(bytes);
    const record = {
      sender: bytesToHex(reader.take(32)),
      claimer: bytesToHex(reader.take(32)),
      amount: reader.u128(),
      hash: bytesToHex(reader.take(32)),
      expiry: reader.u64(),
      claimed: reader.u8() === 1,
      refunded: reader.u8() === 1,
    };
    return reader.rest() === 0 ? record : null;
  } catch {
    return null;
  }
}

export type VaultRecord = {
  creator: string;
  beneficiary: string;
  total: bigint;
  claimed: bigint;
  schedule: string;
  cancelled: boolean;
};

function scheduleText(reader: Reader): string {
  const variant = reader.u32();
  if (variant === 0) return `absolute ${reader.u64().toString()}`;
  if (variant !== 1) throw new Error("schedule");
  const start = reader.u64();
  const end = reader.u64();
  const flag = reader.u8();
  if (flag === 0) return `linear ${start.toString()}..${end.toString()}`;
  if (flag !== 1) throw new Error("cliff");
  return `linear ${start.toString()}..${end.toString()} cliff ${reader.u64().toString()}`;
}

export function decodeVault(bytes: Uint8Array): VaultRecord | null {
  try {
    const reader = new Reader(bytes);
    const record = {
      creator: bytesToHex(reader.take(32)),
      beneficiary: bytesToHex(reader.take(32)),
      total: reader.u128(),
      claimed: reader.u128(),
      schedule: scheduleText(reader),
      cancelled: reader.u8() === 1,
    };
    return reader.rest() === 0 ? record : null;
  } catch {
    return null;
  }
}
