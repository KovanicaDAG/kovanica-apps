import { argon2id } from "hash-wasm";
import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { bytesToHex, hexToBytes, wipe } from "./bytes";

export const KDF_MEMORY_KIB = 65536;
export const KDF_ITERATIONS = 3;
export const KDF_PARALLELISM = 4;

export type VaultKind = "hd" | "legacy";

export type PublicAccount = {
  kind: VaultKind;
  account: number;
  change: number;
  index: number;
  address: string;
  publicKey: string;
  pathLabel: string;
};

export type VaultFile = {
  version: 2;
  kdf: "argon2id";
  kdf_memory_kib: number;
  kdf_iterations: number;
  kdf_parallelism: number;
  cipher: "xchacha20poly1305";
  salt: string;
  nonce: string;
  secret: string;
  kind: VaultKind;
  wordCount: 12 | 24 | null;
  usesBip39Passphrase: boolean;
  accounts: PublicAccount[];
};

export type OpenedVault =
  | { kind: "hd"; entropy: Uint8Array; wordCount: 12 | 24 }
  | { kind: "legacy"; seed: Uint8Array };

async function deriveKey(password: string, salt: Uint8Array): Promise<Uint8Array> {
  const key = await argon2id({
    password,
    salt,
    iterations: KDF_ITERATIONS,
    parallelism: KDF_PARALLELISM,
    memorySize: KDF_MEMORY_KIB,
    hashLength: 32,
    outputType: "binary",
  });
  return key;
}

export function encodeSecret(opened: OpenedVault): Uint8Array {
  if (opened.kind === "legacy") {
    const out = new Uint8Array(1 + 32);
    out[0] = 0x01;
    out.set(opened.seed, 1);
    return out;
  }
  const out = new Uint8Array(2 + opened.entropy.length);
  out[0] = 0x02;
  out[1] = opened.wordCount;
  out.set(opened.entropy, 2);
  return out;
}

export function decodeSecret(plain: Uint8Array): OpenedVault {
  if (plain.length === 32) {
    return { kind: "legacy", seed: plain.slice() };
  }
  if (plain[0] === 0x01 && plain.length === 33) {
    return { kind: "legacy", seed: plain.slice(1) };
  }
  if (plain[0] === 0x02 && (plain[1] === 12 || plain[1] === 24)) {
    const entropy = plain.slice(2);
    if (entropy.length !== (plain[1] === 12 ? 16 : 32)) throw new Error("keystore entropy length mismatch");
    return { kind: "hd", entropy, wordCount: plain[1] };
  }
  throw new Error("keystore payload is not a Kovanica vault");
}

export async function sealVault(
  opened: OpenedVault,
  password: string,
  meta: Omit<VaultFile, "version" | "kdf" | "kdf_memory_kib" | "kdf_iterations" | "kdf_parallelism" | "cipher" | "salt" | "nonce" | "secret">,
): Promise<VaultFile> {
  const salt = new Uint8Array(16);
  const nonce = new Uint8Array(24);
  crypto.getRandomValues(salt);
  crypto.getRandomValues(nonce);
  const key = await deriveKey(password, salt);
  const plain = encodeSecret(opened);
  const cipher = xchacha20poly1305(key, nonce);
  const boxed = cipher.encrypt(plain);
  wipe(key);
  wipe(plain);
  return {
    version: 2,
    kdf: "argon2id",
    kdf_memory_kib: KDF_MEMORY_KIB,
    kdf_iterations: KDF_ITERATIONS,
    kdf_parallelism: KDF_PARALLELISM,
    cipher: "xchacha20poly1305",
    salt: bytesToHex(salt),
    nonce: bytesToHex(nonce),
    secret: bytesToHex(boxed),
    ...meta,
  };
}

export async function openVault(file: VaultFile, password: string): Promise<OpenedVault> {
  if (file.version !== 2 || file.kdf !== "argon2id" || file.cipher !== "xchacha20poly1305") {
    throw new Error("unsupported keystore");
  }
  if (
    file.kdf_memory_kib !== KDF_MEMORY_KIB ||
    file.kdf_iterations !== KDF_ITERATIONS ||
    file.kdf_parallelism !== KDF_PARALLELISM
  ) {
    throw new Error("keystore KDF parameters are not Keystore v2");
  }
  const key = await deriveKey(password, hexToBytes(file.salt));
  let plain: Uint8Array | null = null;
  try {
    plain = xchacha20poly1305(key, hexToBytes(file.nonce)).decrypt(hexToBytes(file.secret));
    const opened = decodeSecret(plain);
    return opened;
  } catch {
    throw new Error("wrong password");
  } finally {
    wipe(key);
    wipe(plain);
  }
}
