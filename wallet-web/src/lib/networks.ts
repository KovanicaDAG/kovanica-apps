export type PresetId = "mainnet" | "testnet" | "hybrid" | "local";

export type NetworkPreset = {
  id: PresetId;
  label: string;
  rpcUrl: string;
  /** null means accept the chain id the node reports */
  expectedChainId: number | null;
  localOnly: boolean;
  /** Mainnet stays visible, but this wallet does not spend on it. */
  frozen?: boolean;
  recommended?: boolean;
};

export const PRESETS: NetworkPreset[] = [
  {
    id: "mainnet",
    label: "Mainnet (1) · read-only",
    rpcUrl: "https://rpc.kovanica.online:8545",
    expectedChainId: 1,
    localOnly: false,
    frozen: true,
  },
  {
    id: "testnet",
    label: "Testnet (2)",
    rpcUrl: "http://127.0.0.1:8545",
    expectedChainId: 2,
    localOnly: false,
    recommended: true,
  },
  {
    id: "hybrid",
    label: "Chain 2 · Hybrid PoW Testnet",
    rpcUrl: "http://127.0.0.1:8545",
    expectedChainId: 2,
    localOnly: true,
  },
  {
    id: "local",
    label: "Local (1337)",
    rpcUrl: "http://127.0.0.1:8545",
    expectedChainId: 1337,
    localOnly: true,
  },
];

export const CHAIN_NAMES: Record<number, string> = {
  1: "Mainnet",
  2: "Testnet",
  3: "Devnet",
  4: "Testnet v2",
  5: "Hybrid PoW",
  1337: "Local",
};

export const FALLBACK_RPC = "https://rpc-testnet.kovanica.online:8545";

export function chainName(chainId: number | null): string {
  if (chainId == null) return "Not probed";
  return CHAIN_NAMES[chainId] ?? `Chain ${chainId}`;
}

export function presetById(id: PresetId): NetworkPreset {
  const preset = PRESETS.find((item) => item.id === id);
  if (!preset) throw new Error("unknown network");
  return preset;
}

const ALLOWED_HOSTS = new Set(["kovanica.online", "rpc.kovanica.online", "testnet.kovanica.online", "rpc-testnet.kovanica.online", "explorer.kovanica.online", "127.0.0.1", "localhost"]);

export function assertRpcUrl(raw: string): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error("RPC URL is not valid");
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") throw new Error("RPC URL must be http or https");
  const host = url.hostname.toLowerCase();
  const allowed =
    ALLOWED_HOSTS.has(host) || (host.endsWith(".kovanica.online") && !host.includes("..")) || isPrivateHost(host);
  if (!allowed) throw new Error("RPC host is not a Kovanica endpoint");
  if (url.username || url.password) throw new Error("RPC URL cannot contain credentials");
  const port = url.port;
  if (port && !["80", "443", "8545"].includes(port)) throw new Error("RPC port is not allowed");
  if ((isPrivateHost(host)) && url.protocol !== "http:") {
    throw new Error("local node URLs use http");
  }
  return url;
}

export function normalizeRpc(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) throw new Error("Enter a host and port");
  const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `http://${trimmed}`;
  return assertRpcUrl(withScheme).toString().replace(/\/$/, "");
}

function isPrivateHost(host: string): boolean {
  if (host === "localhost" || host === "127.0.0.1") return true;
  const parts = host.split(".");
  if (parts.length !== 4 || parts.some((part) => !/^\d+$/.test(part))) return false;
  const [a, b] = parts.map((part) => Number(part));
  if (a === 10) return true;
  if (a === 192 && b === 168) return true;
  return a === 172 && b >= 16 && b <= 31;
}
