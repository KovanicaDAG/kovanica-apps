import { blake3 } from "@noble/hashes/blake3.js";
import { acceptSigned, exportUnsigned } from "./airgap";
import { checkAddress, decodeAddress, encodeAddress } from "./address";
import { asciiPad, bytesToHex, concat, hexToBytes, u32le, u64le, u128le } from "./bytes";
import { buildContractCall, decodeHtlc, decodeTokenState, decodeVault, tokenStateSlot } from "./contracts";
import { bip39Seed, deriveBip44, deriveHardened, entropyFromPhrase, generatePhrase, hdPrivateKey, isValidMnemonic, masterFromSeed } from "./hd";
import { encodeRawTransaction, publicKeyFromSeed, signHash, signingHash } from "./tx";
import { formatRemaining, remainingMs } from "./idle";
import { faucetChain } from "./faucet";
import { quizMatches, quizPositions } from "./quiz";

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
const hashlock = new Uint8Array(32).fill(0x44);
const swapId = new Uint8Array(32).fill(0x11);
const msId = new Uint8Array(32).fill(0x66);
const base = {
  chainId: 1,
  claimer: recipient,
  beneficiary: recipient,
  recipient,
  account: recipient,
  hash: bytesToHex(hashlock),
  secret: "",
  id: bytesToHex(swapId),
  timelockMs: null as bigint | null,
  lockUntilMs: null as bigint | null,
  amountAtoms: null as bigint | null,
  tokenUnits: null as bigint | null,
  txId: null as bigint | null,
  callDataHex: "",
};
const locked = buildContractCall({ ...base, contract: "htlc", method: "htlc_create", timelockMs: 9n, amountAtoms: 1000n });
if (bytesToHex(locked.args) !== bytesToHex(concat([to, u128le(1000n), hashlock, u64le(9n)]))) throw new Error("htlc_create args");
const secret = new TextEncoder().encode("preimage");
const claimedLock = buildContractCall({ ...base, contract: "htlc", method: "htlc_claim", secret: "preimage" });
if (bytesToHex(claimedLock.args) !== bytesToHex(concat([swapId, u64le(BigInt(secret.length)), secret]))) throw new Error("htlc_claim args");
const refunded = buildContractCall({ ...base, contract: "htlc", method: "htlc_refund" });
if (bytesToHex(refunded.args) !== bytesToHex(swapId) || refunded.method !== "htlc_refund") throw new Error("htlc_refund args");

const created = buildContractCall({ ...base, contract: "vault", method: "vault_create", lockUntilMs: 50n, amountAtoms: 5n });
if (bytesToHex(created.args) !== bytesToHex(concat([to, u128le(5n), u32le(0), u64le(50n)]))) throw new Error("vault_create args");
const vaultClaim = buildContractCall({ ...base, contract: "vault", method: "vault_claim" });
if (bytesToHex(vaultClaim.args) !== bytesToHex(swapId) || vaultClaim.method !== "vault_claim") throw new Error("vault_claim args");
const cancelled = buildContractCall({ ...base, contract: "vault", method: "vault_cancel" });
if (bytesToHex(cancelled.args) !== bytesToHex(swapId) || cancelled.method !== "vault_cancel") throw new Error("vault_cancel args");

const proposed = buildContractCall({ ...base, contract: "multisig", method: "multisig_propose", id: bytesToHex(msId), amountAtoms: 42n });
if (bytesToHex(proposed.args) !== bytesToHex(concat([msId, to, u128le(42n), u64le(0n)]))) throw new Error("multisig_propose args");
const confirmed = buildContractCall({ ...base, contract: "multisig", method: "multisig_confirm", id: bytesToHex(msId), txId: 3n });
if (bytesToHex(confirmed.args) !== bytesToHex(concat([msId, u64le(3n)])) || confirmed.method !== "multisig_confirm") throw new Error("multisig_confirm args");
const executed = buildContractCall({ ...base, contract: "multisig", method: "multisig_execute", id: bytesToHex(msId), txId: 3n });
if (executed.method !== "multisig_execute" || bytesToHex(executed.args) !== bytesToHex(confirmed.args)) throw new Error("multisig_execute args");
const moved = buildContractCall({ ...base, contract: "token", method: "token_transfer", tokenUnits: 5n });
if (bytesToHex(moved.args) !== bytesToHex(concat([to, u128le(5n)]))) throw new Error("token_transfer args");
const minted = buildContractCall({ ...base, contract: "token", method: "token_mint", tokenUnits: 8n });
if (bytesToHex(minted.args) !== bytesToHex(concat([to, u128le(8n)]))) throw new Error("token_mint args");
const tokenName = new TextEncoder().encode("A");
const tokenSymbol = new TextEncoder().encode("B");
const tokenState = concat([
  u64le(1n), tokenName,
  u64le(1n), tokenSymbol,
  Uint8Array.of(9),
  u128le(0n),
  new Uint8Array(32),
  new Uint8Array(32),
  u64le(1n),
  to,
  u128le(5n),
  u64le(0n),
  Uint8Array.of(0),
  u64le(0n),
]);
const decoded = decodeTokenState(tokenState);
if (!decoded || decoded.name !== "A" || decoded.symbol !== "B" || decoded.decimals !== 9 || decoded.balances.length !== 1 || decoded.balances[0]!.amount !== 5n) {
  throw new Error("token state decode");
}
if (tokenStateSlot() !== bytesToHex(blake3(new TextEncoder().encode("kvnc/v1/state")))) throw new Error("token state slot");
const htlcBytes = concat([sender, to, u128le(1000n), hashlock, u64le(9n), Uint8Array.of(0), Uint8Array.of(0)]);
const htlcRecord = decodeHtlc(htlcBytes);
if (!htlcRecord || htlcRecord.amount !== 1000n || htlcRecord.expiry !== 9n || htlcRecord.claimed || htlcRecord.refunded) throw new Error("htlc record");
const vaultBytes = concat([sender, to, u128le(5n), u128le(1n), u32le(0), u64le(50n), Uint8Array.of(0)]);
const vaultRecord = decodeVault(vaultBytes);
if (!vaultRecord || vaultRecord.total !== 5n || vaultRecord.claimed !== 1n || vaultRecord.schedule !== "absolute 50" || vaultRecord.cancelled) throw new Error("vault record");

const contractKey = new Uint8Array(32).fill(0x33);
const callTx = {
  sender,
  nonce: 1n,
  fee: 10n,
  kind: { type: "call" as const, contract: contractKey, value: 0n, method: "htlc_refund", args: refunded.args, gasLimit: 100000n },
};
const methodBytes = new TextEncoder().encode("htlc_refund");
const kindBytes = concat([
  Uint8Array.of(4),
  contractKey,
  u64le(0n),
  u64le(BigInt(methodBytes.length)),
  methodBytes,
  u64le(BigInt(refunded.args.length)),
  refunded.args,
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
  method: "htlc_refund",
  argsHex: bytesToHex(refunded.args),
  valueAtoms: "0",
  gasLimit: "100000",
  pathLabel: "vector",
  details: [],
});
if (airCall.wire !== 6 || airCall.tag !== 4) throw new Error("airgap call must use wire 6 and tag 4");
if (remainingMs(1, 0, 30_000) !== 30_000) throw new Error("1m idle left 30s");
if (remainingMs(5, 0, 0) !== 300_000) throw new Error("5m idle is 300s");
if (remainingMs(15, 1_000, 1_000 + 15 * 60_000) !== 0) throw new Error("15m idle expired");
if (remainingMs(60, 0, 1_000) !== 60 * 60_000 - 1_000) throw new Error("60m idle countdown");
if (formatRemaining(61_000) !== "1:01") throw new Error("countdown format");
if (formatRemaining(0) !== "0:00") throw new Error("countdown zero");
if (!faucetChain(5) || !faucetChain(3) || !faucetChain(1337) || faucetChain(1)) throw new Error("faucet chain gate");
const sample = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x"];
if (quizPositions(24).join() !== "3,8,14") throw new Error("24-word quiz positions");
if (quizPositions(12).join() !== "3,8,12") throw new Error("12-word quiz positions");
if (!quizMatches(sample, { 3: "c", 8: "h", 14: "n" })) throw new Error("quiz should accept the right words");
if (quizMatches(sample, { 3: "a", 8: "h", 14: "n" })) throw new Error("quiz should reject a wrong word");

console.log("vectors ok", address);
