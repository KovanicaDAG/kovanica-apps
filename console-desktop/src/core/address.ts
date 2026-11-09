import { blake3 } from "@noble/hashes/blake3.js";

const PREFIX = "kvnc";
const SUFFIX = "dag";
const BODY_CHARS = 57;
export const ADDRESS_LENGTH = PREFIX.length + BODY_CHARS + SUFFIX.length;
const CHECKSUM_BITS = 29;
const ALPHABET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

const LEGACY_HEX_CHAIN_IDS = new Set([3, 4, 1337]);

function keyToBigInt(publicKey: Uint8Array): bigint {
  let n = 0n;
  for (const byte of publicKey) n = (n << 8n) | BigInt(byte);
  return n;
}

function checksum29(publicKey: Uint8Array): bigint {
  const digest = blake3(publicKey);
  const word =
    (BigInt(digest[0]!) << 24n) |
    (BigInt(digest[1]!) << 16n) |
    (BigInt(digest[2]!) << 8n) |
    BigInt(digest[3]!);
  return word >> BigInt(32 - CHECKSUM_BITS);
}

export function encodeAddress(publicKey: Uint8Array): string {
  if (publicKey.length !== 32) throw new Error("address payload must be 32 bytes");
  let n = (keyToBigInt(publicKey) << BigInt(CHECKSUM_BITS)) | checksum29(publicKey);
  const chars = new Array<string>(BODY_CHARS);
  for (let i = BODY_CHARS - 1; i >= 0; i -= 1) {
    chars[i] = ALPHABET[Number(n & 31n)]!;
    n >>= 5n;
  }
  return `${PREFIX}${chars.join("")}${SUFFIX}`;
}

export function decodeAddress(value: string): Uint8Array {
  const trimmed = value.trim().toLowerCase();
  if (!trimmed.startsWith(PREFIX) || !trimmed.endsWith(SUFFIX)) {
    throw new Error("address must start with kvnc and end with dag");
  }
  if (trimmed.length !== ADDRESS_LENGTH) {
    throw new Error(`address must be ${ADDRESS_LENGTH} characters`);
  }
  let n = 0n;
  for (const char of trimmed.slice(PREFIX.length, -SUFFIX.length)) {
    const index = ALPHABET.indexOf(char);
    if (index < 0) throw new Error("address contains a character outside the base32 alphabet");
    n = (n << 5n) | BigInt(index);
  }
  const check = n & ((1n << BigInt(CHECKSUM_BITS)) - 1n);
  const key = n >> BigInt(CHECKSUM_BITS);
  const payload = hexToFixed(key, 32);
  if (check !== checksum29(payload)) throw new Error("address checksum mismatch");
  return payload;
}

function hexToFixed(value: bigint, length: number): Uint8Array {
  const hex = value.toString(16).padStart(length * 2, "0");
  const out = new Uint8Array(length);
  for (let i = 0; i < length; i += 1) out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export type AddressCheck = {
  ok: boolean;
  legacyHex: boolean;
  message: string;
};

function isLegacyHex(value: string): boolean {
  const trimmed = value.trim().toLowerCase();
  if (/^0x[0-9a-f]{64}$/.test(trimmed)) return true;
  if (/^kvnc[0-9a-f]{64}dag$/.test(trimmed)) return true;
  if (/^[0-9a-f]{64}$/.test(trimmed)) return true;
  return false;
}

export function checkAddress(value: string, chainId: number | null): AddressCheck {
  const trimmed = value.trim();
  if (!trimmed) return { ok: false, legacyHex: false, message: "Enter a recipient address" };
  if (isLegacyHex(trimmed)) {
    const allowed = chainId != null && LEGACY_HEX_CHAIN_IDS.has(chainId);
    if (!allowed) {
      return {
        ok: false,
        legacyHex: true,
        message:
          chainId === 1
            ? "Mainnet rejects legacy hex addresses. Use a kvnc…dag address."
            : "This network rejects legacy hex addresses. Use a kvnc…dag address.",
      };
    }
    return { ok: true, legacyHex: true, message: "Legacy hex accepted on this network" };
  }
  try {
    decodeAddress(trimmed.toLowerCase());
    return { ok: true, legacyHex: false, message: "Checksum matches" };
  } catch (error) {
    return {
      ok: false,
      legacyHex: false,
      message: error instanceof Error ? error.message : "Invalid address",
    };
  }
}

export function decodeRecipient(value: string, chainId: number | null): Uint8Array {
  const check = checkAddress(value, chainId);
  if (!check.ok) throw new Error(check.message);
  if (!check.legacyHex) return decodeAddress(value.trim().toLowerCase());
  const trimmed = value.trim().toLowerCase();
  const hex = trimmed.startsWith("kvnc")
    ? trimmed.slice(4, -3)
    : trimmed.startsWith("0x")
      ? trimmed.slice(2)
      : trimmed;
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i += 1) out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}
