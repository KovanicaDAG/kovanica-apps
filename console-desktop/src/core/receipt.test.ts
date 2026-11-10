import { describe, expect, it } from "vitest";

import { decodeLogs, decodeReceipt, formatReceipt, postRpc, probeChain5 } from "./receipt";

describe("execution receipts", () => {
  it("decodes success, revert, and logs", () => {
    const view = decodeReceipt({
      status: "0x1",
      gasUsed: "0x5208",
      blockNumber: "0x10",
      transactionHash: "0xabc",
      logs: [{ topics: ["transfer"], blockNumber: "0x10", transactionHash: "0xabc", data: "0x00", address: "kvnc" }],
    });
    expect(view.status).toBe("success");
    expect(view.gasUsed).toBe("21000");
    expect(view.height).toBe("16");
    expect(view.logs).toHaveLength(1);
    expect(formatReceipt(view)).toContain("Success");
    expect(decodeReceipt({ status: "0x0", error: "locked" }).error).toBe("locked");
    expect(decodeReceipt(null).status).toBe("pending");
    expect(decodeLogs([{ topics: ["vault_create"], blockNumber: "4", transactionHash: "aa" }])[0]?.topics[0]).toBe("vault_create");
  });

  it("fails closed when Chain 5 does not answer", async () => {
    const extra = (globalThis as { process?: { env?: { KOVANICA_CHAIN5_RPC?: string } } }).process?.env?.KOVANICA_CHAIN5_RPC;
    const probe = await probeChain5(extra, 2000);
    if (!probe.reachable || probe.chainId !== 5) {
      expect(probe.error).toBeTruthy();
      return;
    }
    const missing = await postRpc(probe.url, "kovanica_getTransactionReceipt", ["00".repeat(32)]);
    expect(decodeReceipt(missing).status).toBe("pending");
    const logs = decodeLogs(await postRpc(probe.url, "kovanica_getLogs", [{ fromBlock: "0x0", toBlock: "0x20" }]));
    expect(Array.isArray(logs)).toBe(true);
  });
});
