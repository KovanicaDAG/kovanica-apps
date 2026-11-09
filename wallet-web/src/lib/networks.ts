export type PresetId = "mainnet" | "testnet" | "hybrid" | "local";

export type NetworkPreset = {
  id: PresetId;
  label: string;
  rpcUrl: string;
  /** null means accept the chain id the node reports */
  expectedChainId: number | null;
  localOnly: boolean;
};

export const PRESETS: NetworkPreset[] = [
  {
    id: "mainnet",
    label: "Mainnet",
    rpcUrl: "https://rpc.kovanica.online:8545",
    expectedChainId: 1,
    localOnly: false,
  },
  {
    id: "testnet",
    label: "Testnet v2",
    rpcUrl: "https://testnet.kovanica.online:8545",
    expectedChainId: 4,
    localOnly: false,
  },
  {
    id: "hybrid",
    label: "Hybrid PoW",
    rpcUrl: "http://127.0.0.1:8545",
    expectedChainId: 5,
    localOnly: true,
  },
  {
    id: "local",
    label: "Local",
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
    ALLOWED_HOSTS.has(host) || (host.endsWith(".kovanica.online") && !host.includes(".."));
  if (!allowed) throw new Error("RPC host is not a Kovanica endpoint");
  if (url.username || url.password) throw new Error("RPC URL cannot contain credentials");
  const port = url.port;
  if (port && !["80", "443", "8545"].includes(port)) throw new Error("RPC port is not allowed");
  if ((host === "127.0.0.1" || host === "localhost") && url.protocol !== "http:") {
    throw new Error("local node URLs use http");
  }
  return url;
}
