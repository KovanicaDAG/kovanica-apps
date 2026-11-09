import { checkAddress, decodeAddress, encodeAddress } from "./address";
import { bytesToHex, hexToBytes } from "./bytes";
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

console.log("vectors ok", address);
