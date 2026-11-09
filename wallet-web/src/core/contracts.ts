import { decodeRecipient } from "./address";
import { bytesToHex, concat, hexToBytes, u64le } from "./bytes";
import type { PreviewDetail } from "./session";

export type CallDraft = {
  method: string;
  args: Uint8Array;
  details: PreviewDetail[];
};

function key(address: string, chainId: number | null, label: string): Uint8Array {
  const bytes = decodeRecipient(address, chainId);
  if (bytes.length !== 32) throw new Error(`${label} must be 32 bytes`);
  return bytes;
}

function hex32(value: string, label: string): Uint8Array {
  const clean = value.trim().toLowerCase().replace(/^0x/, "");
  if (!/^[0-9a-f]{64}$/.test(clean)) throw new Error(`${label} must be 32 bytes of hex`);
  return hexToBytes(clean);
}

function atoms(value: bigint): Uint8Array {
  return u64le(value);
}

export function buildContractCall(input: {
  contract: "vault" | "htlc" | "multisig" | "token";
  method: string;
  chainId: number | null;
  recipient: string;
  account: string;
  amountAtoms: bigint | null;
  tokenUnits: bigint | null;
  hashlock: string;
  timelockHeight: bigint | null;
  lockId: bigint | null;
  secretPreimage: string;
  proposalId: bigint | null;
  callDataHex: string;
}): CallDraft {
  const method = input.method.trim();
  if (input.contract === "vault" && method === "claim") {
    return { method, args: new Uint8Array(), details: [{ label: "Action", value: "Claim vested KVNC" }] };
  }
  if (input.contract === "vault" && method === "withdraw") {
    if (input.amountAtoms == null) throw new Error("Enter a withdraw amount");
    const recipient = key(input.recipient, input.chainId, "recipient");
    return {
      method,
      args: concat([recipient, atoms(input.amountAtoms)]),
      details: [
        { label: "Recipient", value: input.recipient.trim().toLowerCase(), mono: true },
        { label: "Withdraw", value: `${input.amountAtoms.toString()} atoms` },
      ],
    };
  }
  if (input.contract === "htlc" && method === "lock") {
    if (input.timelockHeight == null) throw new Error("Enter a timelock height");
    const recipient = key(input.recipient, input.chainId, "recipient");
    const hashlock = hex32(input.hashlock, "hashlock");
    return {
      method,
      args: concat([recipient, hashlock, u64le(input.timelockHeight)]),
      details: [
        { label: "Recipient", value: input.recipient.trim().toLowerCase(), mono: true },
        { label: "Hashlock", value: bytesToHex(hashlock), mono: true },
        { label: "Timelock height", value: input.timelockHeight.toString() },
      ],
    };
  }
  if (input.contract === "htlc" && method === "claim") {
    if (input.lockId == null) throw new Error("Enter a lock id");
    const secret = hex32(input.secretPreimage, "secret preimage");
    return {
      method,
      args: concat([u64le(input.lockId), secret]),
      details: [
        { label: "Lock ID", value: input.lockId.toString() },
        { label: "Secret preimage", value: bytesToHex(secret), mono: true },
      ],
    };
  }
  if (input.contract === "htlc" && method === "refund") {
    if (input.lockId == null) throw new Error("Enter a lock id");
    return {
      method,
      args: u64le(input.lockId),
      details: [{ label: "Lock ID", value: input.lockId.toString() }],
    };
  }
  if (input.contract === "multisig" && method === "propose") {
    if (input.amountAtoms == null) throw new Error("Enter an amount");
    const recipient = key(input.recipient, input.chainId, "recipient");
    const extra = input.callDataHex.trim() ? hexToBytes(input.callDataHex.trim().replace(/^0x/, "")) : new Uint8Array();
    return {
      method,
      args: concat([recipient, atoms(input.amountAtoms), u64le(BigInt(extra.length)), extra]),
      details: [
        { label: "Recipient", value: input.recipient.trim().toLowerCase(), mono: true },
        { label: "Proposal amount", value: `${input.amountAtoms.toString()} atoms` },
        { label: "Call data", value: extra.length ? bytesToHex(extra) : "none", mono: extra.length > 0 },
      ],
    };
  }
  if ((input.contract === "multisig" && (method === "approve" || method === "execute"))) {
    if (input.proposalId == null) throw new Error("Enter a proposal id");
    return {
      method,
      args: u64le(input.proposalId),
      details: [{ label: "Proposal ID", value: input.proposalId.toString() }],
    };
  }
  if (input.contract === "token" && method === "transfer") {
    if (input.tokenUnits == null) throw new Error("Enter a token amount");
    const recipient = key(input.recipient, input.chainId, "recipient");
    return {
      method,
      args: concat([recipient, u64le(input.tokenUnits)]),
      details: [
        { label: "Recipient", value: input.recipient.trim().toLowerCase(), mono: true },
        { label: "Token amount", value: input.tokenUnits.toString() },
      ],
    };
  }
  if (input.contract === "token" && (method === "mint" || method === "burn")) {
    if (input.tokenUnits == null) throw new Error("Enter a token amount");
    const account = key(input.account, input.chainId, "account");
    return {
      method,
      args: concat([account, u64le(input.tokenUnits)]),
      details: [
        { label: "Account", value: input.account.trim().toLowerCase(), mono: true },
        { label: "Token amount", value: input.tokenUnits.toString() },
      ],
    };
  }
  throw new Error("That contract action is not one of the typed forms");
}
