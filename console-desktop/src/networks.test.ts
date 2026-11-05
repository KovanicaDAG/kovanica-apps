import { describe, expect, it } from "vitest";

import { NETWORK_PRESETS, presetById, presetFor } from "./networks";

describe("connection network switch", () => {
  it("offers the live chain-4 testnet first", () => {
    expect(NETWORK_PRESETS[0]).toEqual({
      id: "testnet",
      label: "Testnet (Chain 4)",
      chainId: 4,
      rpcUrl: "https://rpc-testnet.kovanica.online/rpc",
    });
  });

  it("keeps mainnet listed and local for development", () => {
    expect(presetById("mainnet")?.chainId).toBe(1);
    expect(presetById("local")?.chainId).toBe(1337);
    expect(presetById("nope")).toBeNull();
  });

  it("matches a saved endpoint including a trailing slash", () => {
    expect(presetFor("https://rpc-testnet.kovanica.online/rpc")?.id).toBe("testnet");
    expect(presetFor("  https://rpc-testnet.kovanica.online/rpc/  ")?.id).toBe("testnet");
    expect(presetFor("http://127.0.0.1:8545/rpc")?.id).toBe("local");
    expect(presetFor("http://localhost:9999/rpc")).toBeNull();
  });
});
