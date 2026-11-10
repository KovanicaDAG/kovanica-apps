import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { blake3 } from "@noble/hashes/blake3.js";
import { encodeAddress } from "./address.ts";
import { bytesToHex, concat, hexToBytes, u32le, u64le, u128le } from "./bytes.ts";
import {
  buildContractCall,
  decodeHtlc,
  decodeMultisigState,
  decodeMultisigTx,
  decodeTokenState,
  decodeVault,
  deriveMultisigId,
  deriveMultisigIdHex,
  multisigTxSlot,
  prefixedSlot,
  tokenStateSlot,
} from "./contracts.ts";

describe("smart contracts & ABI UX", () => {
  const alice = new Uint8Array(32).fill(0x11);
  const bob = new Uint8Array(32).fill(0x22);
  const charlie = new Uint8Array(32).fill(0x33);
  const aliceAddr = encodeAddress(alice);
  const bobAddr = encodeAddress(bob);
  const charlieAddr = encodeAddress(charlie);
  const aliceHex = bytesToHex(alice);
  const bobHex = bytesToHex(bob);
  const charlieHex = bytesToHex(charlie);
  const dummyHash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

  describe("HTLC", () => {
    it("builds htlc_create call args correctly", () => {
      const call = buildContractCall({
        contract: "htlc",
        method: "htlc_create",
        chainId: 5,
        claimer: bobAddr,
        beneficiary: "",
        recipient: "",
        account: "",
        hash: dummyHash,
        secret: "",
        id: "",
        timelockMs: 1700000000000n,
        lockUntilMs: null,
        amountAtoms: 50_000_000n,
        tokenUnits: null,
        txId: null,
        callDataHex: "",
      });
      assert.equal(call.method, "htlc_create");
      const expected = concat([bob, u128le(50_000_000n), hexToBytes(dummyHash), u64le(1700000000000n)]);
      assert.equal(bytesToHex(call.args), bytesToHex(expected));
    });

    it("decodes HTLC state record", () => {
      const recordBytes = concat([
        alice,
        bob,
        u128le(10_000_000n),
        hexToBytes(dummyHash),
        u64le(1800000000000n),
        Uint8Array.of(0), // claimed
        Uint8Array.of(0), // refunded
      ]);
      const record = decodeHtlc(recordBytes);
      assert.ok(record);
      assert.equal(record.sender, aliceHex);
      assert.equal(record.claimer, bobHex);
      assert.equal(record.amount, 10_000_000n);
      assert.equal(record.hash, dummyHash);
      assert.equal(record.expiry, 1800000000000n);
      assert.equal(record.claimed, false);
      assert.equal(record.refunded, false);
    });
  });

  describe("Vault", () => {
    it("builds vault_create call args correctly", () => {
      const call = buildContractCall({
        contract: "vault",
        method: "vault_create",
        chainId: 5,
        claimer: "",
        beneficiary: bobAddr,
        recipient: "",
        account: "",
        hash: "",
        secret: "",
        id: "",
        timelockMs: null,
        lockUntilMs: 1900000000000n,
        amountAtoms: 100_000_000n,
        tokenUnits: null,
        txId: null,
        callDataHex: "",
      });
      assert.equal(call.method, "vault_create");
      const expected = concat([bob, u128le(100_000_000n), u32le(0), u64le(1900000000000n)]);
      assert.equal(bytesToHex(call.args), bytesToHex(expected));
    });

    it("decodes Vault record", () => {
      const vaultBytes = concat([
        alice,
        bob,
        u128le(250_000_000n),
        u128le(50_000_000n),
        u32le(0), // schedule variant 0 (absolute)
        u64le(1900000000000n),
        Uint8Array.of(0), // cancelled
      ]);
      const record = decodeVault(vaultBytes);
      assert.ok(record);
      assert.equal(record.creator, aliceHex);
      assert.equal(record.beneficiary, bobHex);
      assert.equal(record.total, 250_000_000n);
      assert.equal(record.claimed, 50_000_000n);
      assert.equal(record.schedule, "absolute 1900000000000");
      assert.equal(record.cancelled, false);
    });
  });

  describe("Multisig", () => {
    it("derives deterministic MultisigId fingerprint matching PRO-21 specification", () => {
      const owners = [bob, alice, charlie]; // unsorted input
      const id = deriveMultisigId(owners, 2);
      // Expected: blake3( b"multisig/id" || alice || bob || charlie || u32le(2) )
      const sorted = [alice, bob, charlie];
      const expectedBlake = blake3(concat([new TextEncoder().encode("multisig/id"), ...sorted, u32le(2)]));
      assert.equal(bytesToHex(id), bytesToHex(expectedBlake));
      assert.equal(deriveMultisigIdHex(owners, 2), bytesToHex(expectedBlake));
    });

    it("builds multisig_create call args", () => {
      const call = buildContractCall({
        contract: "multisig",
        method: "multisig_create",
        chainId: 5,
        claimer: "",
        beneficiary: "",
        recipient: "",
        account: "",
        hash: "",
        secret: "",
        id: "",
        timelockMs: null,
        lockUntilMs: null,
        amountAtoms: null,
        tokenUnits: null,
        txId: null,
        callDataHex: "",
        threshold: 2,
        owners: `${aliceAddr}, ${bobAddr}`,
      });
      assert.equal(call.method, "multisig_create");
      const expectedOwners = concat([u64le(2n), alice, bob]);
      const expectedArgs = concat([expectedOwners, u32le(2)]);
      assert.equal(bytesToHex(call.args), bytesToHex(expectedArgs));
    });

    it("builds multisig_propose and confirm calls", () => {
      const mid = "1111111111111111111111111111111111111111111111111111111111111111";
      const propose = buildContractCall({
        contract: "multisig",
        method: "multisig_propose",
        chainId: 5,
        claimer: "",
        beneficiary: "",
        recipient: bobAddr,
        account: "",
        hash: "",
        secret: "",
        id: mid,
        timelockMs: null,
        lockUntilMs: null,
        amountAtoms: 10_000_000n,
        tokenUnits: null,
        txId: null,
        callDataHex: "aabb",
      });
      assert.equal(propose.method, "multisig_propose");
      const expectedProposeArgs = concat([
        hexToBytes(mid),
        bob,
        u128le(10_000_000n),
        u64le(2n), // data len
        hexToBytes("aabb"),
      ]);
      assert.equal(bytesToHex(propose.args), bytesToHex(expectedProposeArgs));

      const confirm = buildContractCall({
        contract: "multisig",
        method: "multisig_confirm",
        chainId: 5,
        claimer: "",
        beneficiary: "",
        recipient: "",
        account: "",
        hash: "",
        secret: "",
        id: mid,
        timelockMs: null,
        lockUntilMs: null,
        amountAtoms: null,
        tokenUnits: null,
        txId: 42n,
        callDataHex: "",
      });
      assert.equal(confirm.method, "multisig_confirm");
      assert.equal(bytesToHex(confirm.args), bytesToHex(concat([hexToBytes(mid), u64le(42n)])));
    });

    it("decodes Multisig singleton state", () => {
      const stateBytes = concat([
        u64le(2n), // 2 owners
        alice,
        bob,
        u32le(2), // threshold
        u64le(7n), // next_tx_id
      ]);
      const state = decodeMultisigState(stateBytes);
      assert.ok(state);
      assert.equal(state.owners.length, 2);
      assert.equal(state.owners[0], aliceHex);
      assert.equal(state.owners[1], bobHex);
      assert.equal(state.threshold, 2);
      assert.equal(state.nextTxId, 7n);
    });

    it("decodes Multisig pending transaction record and verifies storage slot", () => {
      const txBytes = concat([
        u64le(3n), // tx_id
        bob, // to
        u128le(5_000_000n), // amount
        u64le(2n), // data len
        hexToBytes("dead"), // data
        u64le(1n), // 1 confirmation
        alice,
        Uint8Array.of(0), // not executed
      ]);
      const tx = decodeMultisigTx(txBytes);
      assert.ok(tx);
      assert.equal(tx.id, 3n);
      assert.equal(tx.to, bobHex);
      assert.equal(tx.amount, 5_000_000n);
      assert.equal(tx.dataHex, "dead");
      assert.equal(tx.confirmations.length, 1);
      assert.equal(tx.confirmations[0], aliceHex);
      assert.equal(tx.executed, false);

      const slot = multisigTxSlot(3n);
      const expectedSlot = bytesToHex(blake3(concat([new TextEncoder().encode("kvnc/v1/mtx/"), u64le(3n)])));
      assert.equal(slot, expectedSlot);
    });
  });

  describe("Token", () => {
    it("verifies token state slot and decodes state", () => {
      assert.equal(tokenStateSlot(), bytesToHex(blake3(new TextEncoder().encode("kvnc/v1/state"))));
      const name = new TextEncoder().encode("Kovanica Wrapped");
      const sym = new TextEncoder().encode("wKVNC");
      const tokenBytes = concat([
        u64le(BigInt(name.length)),
        name,
        u64le(BigInt(sym.length)),
        sym,
        Uint8Array.of(9), // decimals
        u128le(1_000_000_000_000n), // total supply
        alice, // minter
        alice, // pauser
        u64le(1n), // 1 balance entry
        alice,
        u128le(1_000_000_000_000n),
        u64le(0n), // 0 allowances
        Uint8Array.of(0), // not paused
        u64le(0n), // icon len 0
      ]);
      const state = decodeTokenState(tokenBytes);
      assert.ok(state);
      assert.equal(state.name, "Kovanica Wrapped");
      assert.equal(state.symbol, "wKVNC");
      assert.equal(state.decimals, 9);
      assert.equal(state.totalSupply, 1_000_000_000_000n);
      assert.equal(state.balances.length, 1);
      assert.equal(bytesToHex(state.balances[0]!.account), aliceHex);
      assert.equal(state.balances[0]!.amount, 1_000_000_000_000n);
    });
  });
});
