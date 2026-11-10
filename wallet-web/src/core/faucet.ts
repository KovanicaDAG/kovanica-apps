export const FAUCET_ATOMS = "100000000000";
export const FAUCET_COOLDOWN_MS = 60_000;
const KEY = "kvnc.faucet.v1";

/** Chain 5, devnet 3, and local 1337. Mainnet is never included. */
export function faucetChain(chainId: number | null): boolean {
  return chainId === 5 || chainId === 3 || chainId === 1337;
}

export function cooldownRemaining(chainId: number | null, address: string, now: number): number {
  const until = readUntil(chainId, address);
  return Math.max(0, until - now);
}

export function rememberClaim(chainId: number | null, address: string, now: number): void {
  if (chainId == null || typeof localStorage === "undefined") return;
  const all = readAll();
  all[`${chainId}:${address}`] = now + FAUCET_COOLDOWN_MS;
  localStorage.setItem(KEY, JSON.stringify(all));
}

function readUntil(chainId: number | null, address: string): number {
  if (chainId == null) return 0;
  return readAll()[`${chainId}:${address}`] ?? 0;
}

function readAll(): Record<string, number> {
  if (typeof localStorage === "undefined") return {};
  try {
    const parsed = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, number>;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}
