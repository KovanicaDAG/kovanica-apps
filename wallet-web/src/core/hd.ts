import { hmac } from "@noble/hashes/hmac.js";
import { sha512 } from "@noble/hashes/sha2.js";
import { entropyToMnemonic, mnemonicToEntropy, mnemonicToSeedSync, validateMnemonic } from "@scure/bip39";
import { wordlist } from "@scure/bip39/wordlists/english";
import { wipe } from "./bytes";

export const COIN_TYPE = 3007;
const HARDENED = 0x80000000;
const ED25519_HMAC_KEY = new TextEncoder().encode("ed25519 seed");

export type Slip10Node = { key: Uint8Array; chain: Uint8Array };

function hmac512(key: Uint8Array, data: Uint8Array): Uint8Array {
  return hmac(sha512, key, data);
}

export function masterFromSeed(seed: Uint8Array): Slip10Node {
  if (seed.length < 16 || seed.length > 64) throw new Error("SLIP-0010 seed must be 16 to 64 bytes");
  const digest = hmac512(ED25519_HMAC_KEY, seed);
  const node = { key: digest.slice(0, 32), chain: digest.slice(32) };
  wipe(digest);
  return node;
}

export function deriveHardened(parent: Slip10Node, index: number): Slip10Node {
  if (!Number.isInteger(index) || index < HARDENED) {
    throw new Error("Ed25519 derivation rejects a non-hardened step");
  }
  const data = new Uint8Array(37);
  data[0] = 0;
  data.set(parent.key, 1);
  new DataView(data.buffer).setUint32(33, index, false);
  const digest = hmac512(parent.chain, data);
  wipe(data);
  const node = { key: digest.slice(0, 32), chain: digest.slice(32) };
  wipe(digest);
  return node;
}

export function formatBip44(account: number, change: number, index: number): string {
  return `m/44'/${COIN_TYPE}'/${account}'/${change}'/${index}'`;
}

export function deriveBip44(seed64: Uint8Array, account: number, change: number, index: number): Uint8Array {
  for (const step of [account, change, index]) {
    if (!Number.isInteger(step) || step < 0 || step >= HARDENED) throw new Error("index out of range");
  }
  let node = masterFromSeed(seed64);
  for (const step of [44, COIN_TYPE, account, change, index]) {
    const child = deriveHardened(node, step + HARDENED);
    wipe(node.key);
    wipe(node.chain);
    node = child;
  }
  const key = node.key;
  wipe(node.chain);
  return key;
}

export function normalizeMnemonic(phrase: string): string {
  return phrase.trim().toLowerCase().split(/\s+/).filter(Boolean).join(" ");
}

export function mnemonicWordCount(phrase: string): number {
  return normalizeMnemonic(phrase).split(" ").filter(Boolean).length;
}

export function isValidMnemonic(phrase: string): boolean {
  const normalized = normalizeMnemonic(phrase);
  const count = mnemonicWordCount(normalized);
  if (count !== 12 && count !== 24) return false;
  return validateMnemonic(normalized, wordlist);
}

export function generatePhrase(words: 12 | 24): string {
  const entropy = new Uint8Array(words === 12 ? 16 : 32);
  crypto.getRandomValues(entropy);
  const phrase = entropyToMnemonic(entropy, wordlist);
  wipe(entropy);
  return phrase;
}

export function entropyFromPhrase(phrase: string): Uint8Array {
  const normalized = normalizeMnemonic(phrase);
  if (!isValidMnemonic(normalized)) throw new Error("invalid English BIP-39 phrase");
  return mnemonicToEntropy(normalized, wordlist);
}

export function phraseFromEntropy(entropy: Uint8Array): string {
  return entropyToMnemonic(entropy, wordlist);
}

/** 64-byte BIP-39 seed. This is not the legacy 32-byte ADR-0002 key. */
export function bip39Seed(phrase: string, passphrase = ""): Uint8Array {
  const normalized = normalizeMnemonic(phrase);
  if (!isValidMnemonic(normalized)) throw new Error("invalid English BIP-39 phrase");
  return mnemonicToSeedSync(normalized, passphrase);
}

export function hdPrivateKey(
  entropy: Uint8Array,
  bip39Passphrase: string,
  account: number,
  change: number,
  index: number,
): Uint8Array {
  const phrase = phraseFromEntropy(entropy);
  const seed = bip39Seed(phrase, bip39Passphrase);
  const key = deriveBip44(seed, account, change, index);
  wipe(seed);
  return key;
}

export function suggestWords(prefix: string): string[] {
  const text = prefix.trim().toLowerCase();
  if (!text) return [];
  return wordlist.filter((word) => word.startsWith(text)).slice(0, 6);
}

export function wordlistIncludes(word: string): boolean {
  return wordlist.includes(word.trim().toLowerCase());
}
