/**
 * Built-in endpoints behind the Connection switch.
 *
 * Kept in one place so the button ids in `index.html`, the saved endpoint, and
 * the Rust default in `src-tauri/src/connection.rs` cannot drift apart.
 */

export interface NetworkPreset {
  /** Matches the `data-network` (and `#net-<id>`) of the switch button. */
  id: "testnet" | "mainnet" | "local";
  label: string;
  chainId: number;
  rpcUrl: string;
}

/** Chain ids: mainnet 1, public testnet 4, local 1337. */
export const NETWORK_PRESETS: readonly NetworkPreset[] = [
  {
    id: "testnet",
    label: "Testnet (Chain 4)",
    chainId: 4,
    rpcUrl: "https://rpc-testnet.kovanica.online/rpc",
  },
  {
    id: "mainnet",
    label: "Mainnet (Chain 1)",
    chainId: 1,
    rpcUrl: "https://rpc.kovanica.online/rpc",
  },
  {
    id: "local",
    label: "Local (1337)",
    chainId: 1337,
    rpcUrl: "http://127.0.0.1:8545/rpc",
  },
] as const;

/** Drop a trailing slash so `…/rpc` and `…/rpc/` match the same preset. */
function normalize(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

/** The preset whose endpoint is [url], or null when the endpoint is custom. */
export function presetFor(url: string): NetworkPreset | null {
  const wanted = normalize(url);
  return NETWORK_PRESETS.find((preset) => normalize(preset.rpcUrl) === wanted) ?? null;
}

/** Preset by id, used by the switch handlers. */
export function presetById(id: string): NetworkPreset | null {
  return NETWORK_PRESETS.find((preset) => preset.id === id) ?? null;
}
