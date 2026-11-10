import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { decodeLogs, decodeReceipt, formatReceipt, postRpc, probeChain5 } from "./receipt.ts";

const SUCCESS = {
  status: "0x1",
  gasUsed: "0x5208",
  blockNumber: "0x10",
  transactionHash: "0xabc",
  logs: [{ address: "kvnc", topics: ["transfer"], data: "0x00", blockNumber: "0x10", transactionHash: "0xabc" }],
};

describe("execution receipts", () => {
  it("decodes a success receipt with gas, height, and logs", () => {
    const view = decodeReceipt(SUCCESS);
    assert.equal(view.status, "success");
    assert.equal(view.code, "0x1");
    assert.equal(view.gasUsed, "21000");
    assert.equal(view.height, "16");
    assert.equal(view.hash, "abc");
    assert.equal(view.logs.length, 1);
    assert.equal(view.logs[0]?.topics[0], "transfer");
    assert.match(formatReceipt(view), /Success/);
    assert.match(formatReceipt(view), /gasUsed 21000/);
  });

  it("decodes a revert and a still-pending null", () => {
    const revert = decodeReceipt({ status: "0x0", revertReason: "locked" });
    assert.equal(revert.status, "revert");
    assert.equal(revert.code, "0x0");
    assert.equal(revert.error, "locked");
    assert.equal(decodeReceipt(null).status, "pending");
  });

  it("decodes a getLogs payload", () => {
    const logs = decodeLogs([
      { address: "kvnccontract", topics: ["htlc_create"], data: "0x01", blockNumber: "20", transactionHash: "0xdef" },
    ]);
    assert.equal(logs.length, 1);
    assert.equal(logs[0]?.topics[0], "htlc_create");
    assert.equal(logs[0]?.height, "20");
    assert.equal(logs[0]?.hash, "def");
  });

  it("fails closed when Chain 5 is down and decodes a live receipt when it is up", async () => {
    const extra = (globalThis as { process?: { env?: { KOVANICA_CHAIN5_RPC?: string } } }).process?.env?.KOVANICA_CHAIN5_RPC;
    const probe = await probeChain5(extra, 2000);
    if (!probe.reachable || probe.chainId !== 5) {
      assert.equal(probe.chainId, probe.reachable ? probe.chainId : null);
      assert.ok(probe.error);
      assert.notEqual(probe.error, "");
      return;
    }
    const missing = await postRpc(probe.url, "kovanica_getTransactionReceipt", ["00".repeat(32)]);
    assert.equal(decodeReceipt(missing).status, "pending");
    const logs = decodeLogs(await postRpc(probe.url, "kovanica_getLogs", [{ fromBlock: "0x0", toBlock: "0x20" }]));
    assert.ok(Array.isArray(logs));
  });
});
