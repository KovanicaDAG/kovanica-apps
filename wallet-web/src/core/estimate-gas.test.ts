import test from "node:test";
import assert from "node:assert/strict";
import { estimateGas, simulateCall } from "../services/rpc.ts";

test("estimateGas falls back gracefully when RPC is unreachable", async () => {
  const gas = await estimateGas("http://127.0.0.1:59999", {
    from: "kvnc1111111111111111111111111111111111111111111111111111111111dag",
    to: "kvnc2222222222222222222222222222222222222222222222222222222222dag",
    value: 1000,
  });
  assert.equal(gas, 21000n, "fallback gas should be base transfer gas 21,000");
});

test("simulateCall returns empty hex when RPC is unreachable", async () => {
  const res = await simulateCall("http://127.0.0.1:59999", {
    to: "kvnc2222222222222222222222222222222222222222222222222222222222dag",
    data: "0x1234",
  }).catch(() => "0x");
  assert.equal(res, "0x");
});
