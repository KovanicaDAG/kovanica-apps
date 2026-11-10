import { describe, expect, it } from "vitest";
import { estimateGas } from "./receipt";

describe("gas estimation", () => {
  it("falls back to base transfer gas when node is unreachable", async () => {
    const gas = await estimateGas("http://127.0.0.1:59999", {
      from: "kvnc1111111111111111111111111111111111111111111111111111111111dag",
      to: "kvnc2222222222222222222222222222222222222222222222222222222222dag",
      value: 1000,
    });
    expect(gas).toBe(21000n);
  });
});
