import { blake3 } from "@noble/hashes/blake3.js";
import { acceptSigned, exportUnsigned } from "./airgap";
import { checkAddress, decodeAddress, encodeAddress } from "./address";
import { asciiPad, bytesToHex, concat, hexToBytes, u64le } from "./bytes";
import { buildContractCall } from "./contracts";
import { bip39Seed, deriveBip44, deriveHardened, entropyFromPhrase, generatePhrase, hdPrivateKey, isValidMnemonic, masterFromSeed } from "./hd";
import { encodeRawTransaction, publicKeyFromSeed, signHash, signingHash } from "./tx";

const seed = new Uint8Array(32).fill(0x07);
const sender = publicKeyFromSeed(seed);
const expectSender = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
if (bytesToHex(sender) !== expectSender) {
  throw new Error(`sender mismatch ${bytesToHex(sender)}`);
}

const to = new Uint8Array(32).fill(0x22);
const tx = {
  sender,
  nonce: 5n,
  fee: 1000n,
  kind: { type: "transfer" as const, to, amount: 1_000_000n },
};
const hash = signingHash(tx, 2);
const expectHash = "4acde0e0a120d6c03eaa197c082238fefadaf36f4f40bc1364ae8b3faa067798";
if (bytesToHex(hash) !== expectHash) throw new Error(`hash mismatch ${bytesToHex(hash)}`);

const signature = signHash(seed, hash);
const expectSig =
  "2413f8c3a1508e2b1b2809c5cc9cc74982118d47e8a8e9afaab2d2719349724265b8f80d67359b579001b9c4845723edb9530372a79da9f4093d2f7228820d0e";
if (bytesToHex(signature) !== expectSig) throw new Error(`sig mismatch ${bytesToHex(signature)}`);

const raw = encodeRawTransaction(tx, signature, hash);
if (!raw.startsWith(expectSender)) throw new Error("raw missing sender");
if (raw.includes("0x")) throw new Error("raw has 0x");

const address = encodeAddress(sender);
if (address.length !== 64 || !address.startsWith("kvnc") || !address.endsWith("dag")) {
  throw new Error(`address shape ${address}`);
}
const round = encodeAddress(decodeAddress(address));
if (round !== address) throw new Error("address round-trip failed");
const broken = `${address.slice(0, 10)}${address[10] === "q" ? "p" : "q"}${address.slice(11)}`;
let rejected = false;
try {
  decodeAddress(broken);
} catch {
  rejected = true;
}
if (!rejected) throw new Error("bad checksum was accepted");

const phrase = generatePhrase(24);
if (!isValidMnemonic(phrase)) throw new Error("generated phrase invalid");
const entropy = entropyFromPhrase(phrase);
const legacy = entropy.slice();
const hdKey = hdPrivateKey(entropy, "", 0, 0, 0);
if (bytesToHex(legacy) === bytesToHex(hdKey)) throw new Error("legacy and HD keys collided");
const seed64 = bip39Seed(phrase, "");
const again = deriveBip44(seed64, 0, 0, 0);
if (bytesToHex(again) !== bytesToHex(hdKey)) throw new Error("HD path mismatch");

const parent = masterFromSeed(seed64);
let rejectedHardened = false;
try {
  deriveHardened(parent, 1);
} catch {
  rejectedHardened = true;
}
if (!rejectedHardened) throw new Error("non-hardened child was accepted");

if (checkAddress("0x" + "ab".repeat(32), 1).ok) throw new Error("chain 1 accepted 0x");
if (checkAddress(`kvnc${"ab".repeat(32)}dag`, 1).ok) throw new Error("chain 1 accepted kvnc hex");
if (checkAddress("0x" + "11".repeat(32), 5).ok) throw new Error("chain 5 accepted hex");
if (!checkAddress("0x" + "11".repeat(32), 4).ok) throw new Error("chain 4 rejected legacy hex");
if (!checkAddress("0x" + "11".repeat(32), 1337).ok) throw new Error("local rejected legacy hex");

const recipient = encodeAddress(to);
const base = {
  chainId: 1,
  recipient,
  account: recipient,
  amountAtoms: null as bigint | null,
  tokenUnits: null as bigint | null,
  hashlock: "",
  timelockHeight: null as bigint | null,
  lockId: null as bigint | null,
  secretPreimage: "",
  proposalId: null as bigint | null,
  callDataHex: "",
};
const withdraw = buildContractCall({ ...base, contract: "vault", method: "withdraw", amountAtoms: 1000n });
if (bytesToHex(withdraw.args) !== bytesToHex(concat([to, u64le(1000n)]))) throw new Error("vault withdraw args");
const claimed = buildContractCall({ ...base, contract: "vault", method: "claim" });
if (claimed.args.length !== 0) throw new Error("vault claim should have no args");

const hashlock = new Uint8Array(32).fill(0x44);
const locked = buildContractCall({ ...base, contract: "htlc", method: "lock", hashlock: bytesToHex(hashlock), timelockHeight: 9n });
if (bytesToHex(locked.args) !== bytesToHex(concat([to, hashlock, u64le(9n)]))) throw new Error("htlc lock args");
const secret = new Uint8Array(32).fill(0x55);
const claimedLock = buildContractCall({ ...base, contract: "htlc", method: "claim", lockId: 7n, secretPreimage: bytesToHex(secret) });
if (bytesToHex(claimedLock.args) !== bytesToHex(concat([u64le(7n), secret]))) throw new Error("htlc claim args");
const refunded = buildContractCall({ ...base, contract: "htlc", method: "refund", lockId: 7n });
if (bytesToHex(refunded.args) !== bytesToHex(u64le(7n))) throw new Error("htlc refund args");

const proposed = buildContractCall({ ...base, contract: "multisig", method: "propose", amountAtoms: 42n });
if (bytesToHex(proposed.args) !== bytesToHex(concat([to, u64le(42n), u64le(0n)]))) throw new Error("multisig propose args");
const approved = buildContractCall({ ...base, contract: "multisig", method: "approve", proposalId: 3n });
if (bytesToHex(approved.args) !== bytesToHex(u64le(3n))) throw new Error("multisig approve args");
const moved = buildContractCall({ ...base, contract: "token", method: "transfer", tokenUnits: 5n });
if (bytesToHex(moved.args) !== bytesToHex(concat([to, u64le(5n)]))) throw new Error("token transfer args");
const minted = buildContractCall({ ...base, contract: "token", method: "mint", tokenUnits: 8n });
if (bytesToHex(minted.args) !== bytesToHex(concat([to, u64le(8n)]))) throw new Error("token mint args");

const contractKey = new Uint8Array(32).fill(0x33);
const callTx = {
  sender,
  nonce: 1n,
  fee: 10n,
  kind: { type: "call" as const, contract: contractKey, value: 0n, method: "withdraw", args: withdraw.args, gasLimit: 100000n },
};
const methodBytes = new TextEncoder().encode("withdraw");
const kindBytes = concat([
  Uint8Array.of(4),
  contractKey,
  u64le(0n),
  u64le(BigInt(methodBytes.length)),
  methodBytes,
  u64le(BigInt(withdraw.args.length)),
  withdraw.args,
  u64le(100000n),
]);
if (kindBytes[0] !== 4) throw new Error("signing kind tag is not 4");
const preimage = concat([asciiPad("KUNA/tx/v1", 16), u64le(2n), sender, u64le(1n), kindBytes, u64le(10n)]);
const keyed = blake3(preimage, { key: asciiPad("KUNA-TX-v1", 32) });
if (bytesToHex(keyed) !== bytesToHex(signingHash(callTx, 2))) throw new Error("call sighash mismatch");
const rawCall = encodeRawTransaction(callTx, signature, keyed);
const wire = hexToBytes(rawCall).slice(40, 44);
if (bytesToHex(wire) !== "06000000") throw new Error(`wire kind ${bytesToHex(wire)}`);

const airTransfer = {
  v: 1 as const,
  role: "signed" as const,
  kind: "transfer" as const,
  wire: 0 as const,
  tag: 0 as const,
  chainId: 2,
  networkName: "vector",
  from: address,
  to: encodeAddress(to),
  amount: "1000000",
  fee: "1000",
  nonce: "5",
  method: "",
  args: "",
  value: "0",
  gas: "0",
  sig: bytesToHex(signature),
  raw,
};
const accepted = acceptSigned(JSON.stringify(airTransfer));
if (accepted.hash !== expectHash || accepted.raw !== raw) throw new Error("airgap transfer import mismatch");
const airCall = exportUnsigned({
  kind: "call",
  sender: address,
  counterparty: encodeAddress(contractKey),
  amountAtoms: "0",
  feeAtoms: "10",
  nonce: "1",
  chainId: 2,
  networkName: "vector",
  method: "withdraw",
  argsHex: bytesToHex(withdraw.args),
  valueAtoms: "0",
  gasLimit: "100000",
  pathLabel: "vector",
  details: [],
});
if (airCall.wire !== 6 || airCall.tag !== 4) throw new Error("airgap call must use wire 6 and tag 4");

console.log("vectors ok", address);
