import { sha512 } from "@noble/hashes/sha2.js";
import { blake3 } from "@noble/hashes/blake3.js";
import * as ed from "@noble/ed25519";
import { asciiPad, bytesToHex, concat, equalBytes, u32le, u64le, wipe } from "./bytes";

if ((ed as any).hashes) {
  (ed as any).hashes.sha512 = sha512;
}
if ((ed as any).etc) {
  (ed as any).etc.sha512Sync = (...messages: Uint8Array[]) => sha512(concat(messages));
}

const TX_DOMAIN = asciiPad("KUNA/tx/v1", 16);
const TX_KEY = asciiPad("KUNA-TX-v1", 32);

export type TxKind =
  | { type: "transfer"; to: Uint8Array; amount: bigint }
  | {
      type: "call";
      contract: Uint8Array;
      value: bigint;
      method: string;
      args: Uint8Array;
      gasLimit: bigint;
    };

export type UnsignedTx = {
  sender: Uint8Array;
  nonce: bigint;
  kind: TxKind;
  fee: bigint;
};

function signingKind(kind: TxKind): Uint8Array {
  if (kind.type === "transfer") {
    return concat([Uint8Array.of(0), kind.to, u64le(kind.amount)]);
  }
  const methodBytes = new TextEncoder().encode(kind.method);
  if (methodBytes.length === 0) throw new Error("method must not be empty");
  return concat([
    Uint8Array.of(4),
    kind.contract,
    u64le(kind.value),
    u64le(BigInt(methodBytes.length)),
    methodBytes,
    u64le(BigInt(kind.args.length)),
    kind.args,
    u64le(kind.gasLimit),
  ]);
}

function wireKind(kind: TxKind): Uint8Array {
  if (kind.type === "transfer") {
    return concat([u32le(0), kind.to, u64le(kind.amount)]);
  }
  const methodBytes = new TextEncoder().encode(kind.method);
  return concat([
    u32le(6),
    kind.contract,
    u64le(kind.value),
    u64le(BigInt(methodBytes.length)),
    methodBytes,
    u64le(BigInt(kind.args.length)),
    kind.args,
    u64le(kind.gasLimit),
  ]);
}

export function signingHash(tx: UnsignedTx, chainId: number): Uint8Array {
  if (!Number.isInteger(chainId) || chainId < 0) throw new Error("chain id must be a non-negative integer");
  const preimage = concat([
    TX_DOMAIN,
    u64le(BigInt(chainId)),
    tx.sender,
    u64le(tx.nonce),
    signingKind(tx.kind),
    u64le(tx.fee),
  ]);
  return blake3(preimage, { key: TX_KEY });
}

export function encodeRawTransaction(tx: UnsignedTx, signature: Uint8Array, hash: Uint8Array): string {
  if (signature.length !== 64) throw new Error("signature must be 64 bytes");
  if (hash.length !== 32) throw new Error("hash must be 32 bytes");
  const body = concat([
    tx.sender,
    u64le(tx.nonce),
    wireKind(tx.kind),
    u64le(tx.fee),
    u64le(64n),
    signature,
    hash,
  ]);
  return bytesToHex(body);
}

export function publicKeyFromSeed(seed: Uint8Array): Uint8Array {
  if (seed.length !== 32) throw new Error("seed must be 32 bytes");
  return ed.getPublicKey(seed);
}

export function signHash(seed: Uint8Array, hash: Uint8Array): Uint8Array {
  if (seed.length !== 32) throw new Error("seed must be 32 bytes");
  if (hash.length !== 32) throw new Error("hash must be 32 bytes");
  const signature = ed.sign(hash, seed);
  return signature;
}

export function signTransaction(tx: UnsignedTx, chainId: number, seed: Uint8Array): { raw: string; hash: string; signature: Uint8Array } {
  const pub = publicKeyFromSeed(seed);
  if (!equalBytes(pub, tx.sender)) {
    wipe(pub);
    throw new Error("signing key does not match the sender");
  }
  wipe(pub);
  const hash = signingHash(tx, chainId);
  const signature = signHash(seed, hash);
  const raw = encodeRawTransaction(tx, signature, hash);
  const hashHex = bytesToHex(hash);
  wipe(hash);
  return { raw, hash: hashHex, signature };
}
