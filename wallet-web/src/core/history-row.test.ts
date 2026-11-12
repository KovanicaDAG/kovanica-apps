import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { callNeed, contractFamily, faucetQuick, historyHeading, receiptStatusLine, shortfall } from "./history-row.ts";

describe("contract history rows", () => {
  it("names the contract family and the receipt", () => {
    assert.equal(contractFamily("token_transfer"), "Token");
    assert.equal(contractFamily("htlc_create"), "HTLC");
    assert.equal(contractFamily("vault_claim"), "Vault");
    assert.equal(contractFamily("multisig_execute"), "Multisig");
    assert.equal(historyHeading("contract", "token_mint"), "Token");
    assert.equal(historyHeading("out", ""), "Sent");
    assert.equal(historyHeading("in", ""), "Received");
    assert.equal(receiptStatusLine("0x1", "confirmed", null), "0x1 Success");
    assert.equal(receiptStatusLine("0x0", "failed", "locked"), "0x0 Reverted: locked");
    assert.equal(receiptStatusLine(null, "pending", null), "Pending");
  });

  it("counts value, fee, and the vault or htlc balance pull", () => {
    assert.equal(callNeed("htlc_create", "0", "1000", "5000"), 6000n);
    assert.equal(callNeed("token_transfer", "0", "1000", "5000"), 1000n);
    assert.equal(shortfall(4000n, 6000n), 2000n);
    assert.equal(shortfall(6000n, 6000n), 0n);
    assert.equal(faucetQuick(2), true);
    assert.equal(faucetQuick(1337), true);
    assert.equal(faucetQuick(1), false);
    assert.equal(faucetQuick(4), false);
  });
});
