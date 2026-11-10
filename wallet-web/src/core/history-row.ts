/** Native atoms an HTLC create or vault deposit pulls from the caller balance. */
export function nativePull(method: string, amount: string): bigint {
  if (method !== "htlc_create" && method !== "vault_create") return 0n;
  return whole(amount);
}

export function whole(text: string): bigint {
  const trimmed = text.trim();
  return /^\d+$/.test(trimmed) ? BigInt(trimmed) : 0n;
}

/** Attached call value, plus the quoted fee, plus any balance pull. */
export function callNeed(method: string, valueAtoms: string, feeAtoms: string, amountAtoms: string): bigint {
  return whole(valueAtoms) + whole(feeAtoms) + nativePull(method, amountAtoms);
}

export function shortfall(balance: bigint, need: bigint): bigint {
  return balance >= need ? 0n : need - balance;
}

/** Inline faucet on the contract form. Chain 2 and local 1337 only. */
export function faucetQuick(chainId: number | null): boolean {
  return chainId === 2 || chainId === 1337;
}

export function contractFamily(method: string): "Token" | "HTLC" | "Vault" | "Multisig" | null {
  if (method.startsWith("token_")) return "Token";
  if (method.startsWith("htlc_")) return "HTLC";
  if (method.startsWith("vault_")) return "Vault";
  if (method.startsWith("multisig_")) return "Multisig";
  return null;
}

export function historyHeading(direction: string, method: string): string {
  const family = contractFamily(method);
  if (direction === "contract") return family ?? "Contract";
  if (family) return family;
  if (direction === "in" || direction === "received") return "Received";
  if (direction === "self") return "Self";
  return "Sent";
}

export function receiptStatusLine(code: string | null | undefined, status: string, error: string | null | undefined): string {
  if (code === "0x0" || status === "failed" || status === "revert" || status === "reverted") {
    return error ? `0x0 Reverted: ${error}` : "0x0 Reverted";
  }
  if (code === "0x1" || status === "confirmed" || status === "success") return "0x1 Success";
  return "Pending";
}

/** First integer in a review row such as `1000 atoms`. */
export function leadingAtoms(value: string): string {
  const match = /^(\d+)/.exec(value.trim());
  return match?.[1] ?? "0";
}
