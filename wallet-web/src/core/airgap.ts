import * as ed from "@noble/ed25519";
import { decodeAddress, decodeRecipient } from "./address";
import { bytesToHex, hexToBytes } from "./bytes";
import type { TxPreview } from "./session";
import { encodeRawTransaction, signingHash, type UnsignedTx } from "./tx";

export type AirgapDoc = {
  v: 1;
  role: "unsigned" | "signed";
  kind: "transfer" | "call";
  wire: 0 | 6;
  tag: 0 | 4;
  chainId: number;
  networkName: string;
  from: string;
  to: string;
  amount: string;
  fee: string;
  nonce: string;
  method: string;
  args: string;
  value: string;
  gas: string;
  sig?: string;
  raw?: string;
};

export function exportUnsigned(preview: TxPreview): AirgapDoc {
  return {
    v: 1,
    role: "unsigned",
    kind: preview.kind,
    wire: preview.kind === "transfer" ? 0 : 6,
    tag: preview.kind === "transfer" ? 0 : 4,
    chainId: preview.chainId,
    networkName: preview.networkName,
    from: preview.sender,
    to: preview.counterparty,
    amount: preview.amountAtoms,
    fee: preview.feeAtoms,
    nonce: preview.nonce,
    method: preview.method,
    args: preview.argsHex,
    value: preview.valueAtoms,
    gas: preview.gasLimit,
  };
}

function asDoc(value: unknown): AirgapDoc {
  if (!value || typeof value !== "object") throw new Error("This is not a Kovanica air-gap payload");
  const doc = value as Partial<AirgapDoc>;
  if (doc.v !== 1 || (doc.role !== "unsigned" && doc.role !== "signed")) throw new Error("This is not a Kovanica air-gap payload");
  if (doc.kind !== "transfer" && doc.kind !== "call") throw new Error("Unknown transaction kind");
  if (typeof doc.from !== "string" || typeof doc.to !== "string") throw new Error("The payload is missing an address");
  if (typeof doc.fee !== "string" || typeof doc.nonce !== "string" || typeof doc.amount !== "string") throw new Error("The payload is missing an amount");
  if (typeof doc.chainId !== "number" || !Number.isInteger(doc.chainId)) throw new Error("The payload is missing a chain id");
  const chainId = doc.chainId;
  return {
    v: 1,
    role: doc.role,
    kind: doc.kind,
    wire: doc.kind === "transfer" ? 0 : 6,
    tag: doc.kind === "transfer" ? 0 : 4,
    chainId,
    networkName: typeof doc.networkName === "string" ? doc.networkName : "",
    from: doc.from,
    to: doc.to,
    amount: doc.amount,
    fee: doc.fee,
    nonce: doc.nonce,
    method: typeof doc.method === "string" ? doc.method : "",
    args: typeof doc.args === "string" ? doc.args : "",
    value: typeof doc.value === "string" ? doc.value : "0",
    gas: typeof doc.gas === "string" ? doc.gas : "0",
    sig: typeof doc.sig === "string" ? doc.sig : undefined,
    raw: typeof doc.raw === "string" ? doc.raw : undefined,
  };
}

export function parseAirgap(text: string): AirgapDoc {
  return asDoc(JSON.parse(text));
}

function unsignedTx(doc: AirgapDoc): UnsignedTx {
  if (doc.kind === "transfer") {
    if (doc.wire !== 0 || doc.tag !== 0) throw new Error("A transfer must use wire index 0");
    return {
      sender: decodeAddress(doc.from),
      nonce: BigInt(doc.nonce),
      fee: BigInt(doc.fee),
      kind: { type: "transfer", to: decodeRecipient(doc.to, doc.chainId), amount: BigInt(doc.amount) },
    };
  }
  if (doc.wire !== 6 || doc.tag !== 4) throw new Error("A contract call must use wire index 6");
  if (!doc.method) throw new Error("A contract call needs a method");
  return {
    sender: decodeAddress(doc.from),
    nonce: BigInt(doc.nonce),
    fee: BigInt(doc.fee),
    kind: {
      type: "call",
      contract: decodeRecipient(doc.to, doc.chainId),
      value: BigInt(doc.value),
      method: doc.method,
      args: doc.args ? hexToBytes(doc.args) : new Uint8Array(),
      gasLimit: BigInt(doc.gas),
    },
  };
}

export function airgapHash(doc: AirgapDoc): Uint8Array {
  return signingHash(unsignedTx(doc), doc.chainId);
}

function signatureFromRaw(rawHex: string): Uint8Array {
  const raw = hexToBytes(rawHex.replace(/^0x/, ""));
  if (raw.length < 104) throw new Error("The raw transaction is too short");
  const length = raw.slice(raw.length - 104, raw.length - 96);
  if (length[0] !== 64 || length.slice(1).some((byte) => byte !== 0)) throw new Error("The raw transaction does not carry a 64-byte signature");
  return raw.slice(raw.length - 96, raw.length - 32);
}

export function acceptSigned(text: string): { raw: string; hash: string; doc: AirgapDoc } {
  const doc = parseAirgap(text);
  if (doc.role !== "signed" || !doc.raw) throw new Error("This payload is not signed yet");
  const tx = unsignedTx(doc);
  const hash = signingHash(tx, doc.chainId);
  const sig = doc.sig ? hexToBytes(doc.sig) : signatureFromRaw(doc.raw);
  if (sig.length !== 64) throw new Error("The signature is not 64 bytes");
  if (!ed.verify(sig, hash, decodeAddress(doc.from))) throw new Error("The signature does not match this transaction");
  const raw = encodeRawTransaction(tx, sig, hash);
  if (raw !== doc.raw.replace(/^0x/, "").toLowerCase()) throw new Error("The raw transaction does not match the reviewed fields");
  return { raw, hash: bytesToHex(hash), doc };
}
