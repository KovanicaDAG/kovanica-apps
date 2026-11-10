import { describe, expect, it } from "vitest";

import { callNeed, faucetQuick, historyHeading, receiptStatusLine, shortfall } from "./history-row";

describe("contract history rows", () => {
  it("names families, receipts, and the balance a call needs", () => {
    expect(historyHeading("contract", "vault_create")).toBe("Vault");
    expect(historyHeading("out", "")).toBe("Sent");
    expect(receiptStatusLine("0x0", "revert", "locked")).toBe("0x0 Reverted: locked");
    expect(receiptStatusLine("0x1", "success", null)).toBe("0x1 Success");
    expect(callNeed("vault_create", "10", "5", "100")).toBe(115n);
    expect(shortfall(20n, 115n)).toBe(95n);
    expect(faucetQuick(5)).toBe(true);
    expect(faucetQuick(1337)).toBe(true);
    expect(faucetQuick(3)).toBe(false);
  });
});
