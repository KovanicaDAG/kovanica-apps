import { invoke } from "@tauri-apps/api/core";

import { checkAddress, decodeRecipient } from "./core/address";
import { acceptSigned } from "./core/airgap";
import { bytesToHex, equalBytes, hexToBytes } from "./core/bytes";
import { callNeed, faucetQuick, shortfall } from "./core/history-row";
import { pushOutbox, updateOutbox } from "./core/outbox";
import { decodeReceipt, formatReceipt } from "./core/receipt";
import {
  buildContractCall,
  decodeTokenState,
  decodeVault,
  prefixedSlot,
  storageBytes,
  tokenStateSlot,
  type ContractName,
} from "./core/contracts";

type Quote = {
  chainId: string;
  nonce: string;
  feeAtoms: string;
  feeDisplay: string;
  address: string;
};

type Action = { id: string; label: string; sign: boolean };

const ACTIONS: Record<ContractName, Action[]> = {
  token: [
    { id: "balance_of", label: "balance_of", sign: false },
    { id: "token_transfer", label: "transfer", sign: true },
    { id: "token_mint", label: "mint", sign: true },
  ],
  htlc: [
    { id: "htlc_create", label: "create", sign: true },
    { id: "htlc_claim", label: "claim", sign: true },
    { id: "htlc_refund", label: "refund", sign: true },
  ],
  vault: [
    { id: "vault_create", label: "deposit", sign: true },
    { id: "vault_claim", label: "withdraw", sign: true },
    { id: "status", label: "status", sign: false },
  ],
  multisig: [
    { id: "multisig_propose", label: "propose", sign: true },
    { id: "multisig_confirm", label: "confirm", sign: true },
    { id: "multisig_execute", label: "execute", sign: true },
  ],
};

type Pending = {
  method: string;
  argsHex: string;
  rows: { label: string; value: string }[];
  contract: string;
  value: string;
  gas: string;
  quote: Quote;
};

function required<T extends Element>(id: string): T {
  const node = document.querySelector<T>(`#${id}`);
  if (!node) throw new Error(`missing #${id}`);
  return node;
}

function messageFrom(error: unknown): string {
  return error instanceof Error ? error.message : "The contract call failed.";
}

function chainNumber(shown: string | null): number | null {
  if (!shown || !/^\d+$/.test(shown)) return null;
  return Number(shown);
}

function atoms(value: string, label: string): bigint {
  const text = value.trim();
  if (!/^\d+$/.test(text)) throw new Error(`${label} must be a whole number of atoms.`);
  return BigInt(text);
}

function needs(id: string, name: string): boolean {
  if (name === "owner") return id === "balance_of" || id === "token_mint";
  if (name === "recipient") return id === "token_transfer" || id === "htlc_create" || id === "vault_create" || id === "multisig_propose";
  if (name === "amount") return id === "token_transfer" || id === "token_mint" || id === "htlc_create" || id === "vault_create" || id === "multisig_propose";
  if (name === "hash") return id === "htlc_create";
  if (name === "timelock") return id === "htlc_create" || id === "vault_create";
  if (name === "secret") return id === "htlc_claim";
  if (name === "record") return id === "htlc_claim" || id === "htlc_refund" || id === "vault_claim" || id === "status" || id.startsWith("multisig_");
  if (name === "proposal") return id === "multisig_confirm" || id === "multisig_execute";
  if (name === "data") return id === "multisig_propose";
  return false;
}

export function mountContracts(shownChainId: () => string | null): void {
  if (!document.querySelector("#contract-panel")) return;
  const kind = required<HTMLSelectElement>("contract-kind");
  const action = required<HTMLSelectElement>("contract-action");
  const methodLine = required<HTMLParagraphElement>("contract-method");
  const contract = required<HTMLInputElement>("contract-address");
  const owner = required<HTMLInputElement>("contract-owner");
  const recipient = required<HTMLInputElement>("contract-recipient");
  const amount = required<HTMLInputElement>("contract-amount");
  const hash = required<HTMLInputElement>("contract-hash");
  const timelock = required<HTMLInputElement>("contract-timelock");
  const secret = required<HTMLInputElement>("contract-secret");
  const record = required<HTMLInputElement>("contract-record");
  const proposal = required<HTMLInputElement>("contract-proposal");
  const data = required<HTMLInputElement>("contract-data");
  const value = required<HTMLInputElement>("contract-value");
  const gas = required<HTMLInputElement>("contract-gas");
  const hint = required<HTMLParagraphElement>("contract-hint");
  const errorText = required<HTMLParagraphElement>("contract-error");
  const readout = required<HTMLParagraphElement>("contract-readout");
  const reviewButton = required<HTMLButtonElement>("contract-review");
  const readButton = required<HTMLButtonElement>("contract-read");
  const reviewBox = required<HTMLDivElement>("contract-review-box");
  const rows = required<HTMLDListElement>("contract-rows");
  const mainnetLabel = required<HTMLLabelElement>("contract-mainnet-label");
  const mainnet = required<HTMLInputElement>("contract-mainnet");
  const signButton = required<HTMLButtonElement>("contract-sign");
  const copyButton = required<HTMLButtonElement>("contract-copy");
  const signed = required<HTMLTextAreaElement>("contract-signed");
  const importButton = required<HTMLButtonElement>("contract-import");
  const status = required<HTMLParagraphElement>("contract-status");
  const receipt = required<HTMLParagraphElement>("contract-receipt");
  const funds = required<HTMLParagraphElement>("contract-funds");
  const faucet = required<HTMLButtonElement>("contract-faucet");
  const fields: Record<string, HTMLElement> = {
    owner: required<HTMLElement>("field-owner"),
    recipient: required<HTMLElement>("field-recipient"),
    amount: required<HTMLElement>("field-amount"),
    hash: required<HTMLElement>("field-hash"),
    timelock: required<HTMLElement>("field-timelock"),
    secret: required<HTMLElement>("field-secret"),
    record: required<HTMLElement>("field-record"),
    proposal: required<HTMLElement>("field-proposal"),
    data: required<HTMLElement>("field-data"),
    value: required<HTMLElement>("field-value"),
  };

  let pending: Pending | null = null;

  function showError(message: string): void {
    errorText.hidden = false;
    errorText.textContent = message;
  }

  function clearError(): void {
    errorText.hidden = true;
    errorText.textContent = "";
  }

  function selectedKind(): ContractName {
    const valueKind = kind.value;
    if (valueKind === "htlc" || valueKind === "vault" || valueKind === "multisig" || valueKind === "token") return valueKind;
    return "token";
  }

  function selectedAction(): Action {
    const list = ACTIONS[selectedKind()];
    return list.find((item) => item.id === action.value) ?? list[0]!;
  }

  function paintActions(): void {
    const list = ACTIONS[selectedKind()];
    const previous = action.value;
    action.replaceChildren();
    for (const item of list) {
      const option = document.createElement("option");
      option.value = item.id;
      option.textContent = item.label;
      action.append(option);
    }
    action.value = list.some((item) => item.id === previous) ? previous : list[0]!.id;
    paintFields();
  }

  function paintFields(): void {
    const current = selectedAction();
    for (const [name, node] of Object.entries(fields)) {
      node.hidden = name === "value" ? !current.sign : !needs(current.id, name);
    }
    methodLine.textContent = current.sign ? `Signed method: ${current.id}` : "Read via kovanica_getStorageAt";
    reviewButton.hidden = !current.sign;
    readButton.hidden = current.sign;
    hint.textContent = current.id === "vault_claim"
      ? "Withdraw claims the vested balance. The contract does not take an amount."
      : current.id === "htlc_create" || current.id === "vault_create"
        ? "The contract pulls Amount from your balance. Leave attached value at 0 unless the call also pays Call.value."
        : "";
    const timelockLabel = document.querySelector("label[for='contract-timelock']");
    if (timelockLabel) timelockLabel.textContent = current.id === "vault_create" ? "Lock until (unix ms)" : "Timelock (unix ms)";
    const recordLabel = document.querySelector("label[for='contract-record']");
    if (recordLabel) {
      recordLabel.textContent = selectedKind() === "multisig"
        ? "Multisig id (32-byte hex)"
        : selectedKind() === "htlc"
          ? "Swap id (32-byte hex)"
          : "Vault id (32-byte hex)";
    }
  }

  function unsignedDoc(): string {
    if (!pending) throw new Error("Review the call first.");
    return JSON.stringify({
      v: 1,
      role: "unsigned",
      kind: "call",
      wire: 6,
      tag: 4,
      chainId: Number(pending.quote.chainId),
      networkName: `chain ${pending.quote.chainId}`,
      from: pending.quote.address,
      to: pending.contract,
      amount: pending.value,
      fee: pending.quote.feeAtoms,
      nonce: pending.quote.nonce,
      method: pending.method,
      args: pending.argsHex,
      value: pending.value,
      gas: pending.gas,
    });
  }

  async function broadcast(raw: string): Promise<void> {
    const hash = await invoke<string>("wallet_broadcast_raw", { raw });
    status.textContent = `Transaction broadcast successful! TxHash: ${hash}`;
    if (pending) {
      pushOutbox({
        hash: hash.replace(/^0x/, ""),
        method: pending.method,
        counterparty: pending.contract,
        status: "pending",
        code: null,
        gasUsed: null,
        error: null,
        events: [],
      });
    }
    await watchReceipt(hash);
  }

  async function watchReceipt(hash: string): Promise<void> {
    receipt.hidden = false;
    receipt.textContent = `status Pending\ntx ${hash.replace(/^0x/, "")}`;
    let left = 6;
    while (left > 0) {
      left -= 1;
      try {
        const raw = await invoke<string>("wallet_tx_receipt", { hash });
        const view = decodeReceipt(JSON.parse(raw) as unknown);
        if (!view.hash) view.hash = hash.replace(/^0x/, "");
        receipt.textContent = formatReceipt(view);
        updateOutbox(hash.replace(/^0x/, ""), {
          status: view.status === "success" ? "success" : view.status === "revert" ? "revert" : "pending",
          code: view.code,
          gasUsed: view.gasUsed,
          error: view.error,
          events: view.logs.map((log) => log.topics[0] || "event").slice(0, 4),
        });
        if (view.status !== "pending") return;
      } catch (error) {
        receipt.textContent = messageFrom(error);
        return;
      }
      await new Promise((resolve) => window.setTimeout(resolve, 5_000));
    }
  }

  async function paintFunds(method: string, valueAtoms: string, amountAtoms: string, feeAtoms: string, chainId: string): Promise<void> {
    funds.hidden = true;
    faucet.hidden = true;
    const reported = await invoke<{ atoms: string }>("wallet_balance");
    const balance = /^\d+$/.test(reported.atoms) ? BigInt(reported.atoms) : 0n;
    const missing = shortfall(balance, callNeed(method, valueAtoms, feeAtoms, amountAtoms));
    if (missing === 0n) return;
    funds.hidden = false;
    funds.textContent = `Balance is short by ${missing.toString()} atoms for this call value and fee.`;
    const chain = Number(chainId);
    faucet.hidden = !faucetQuick(Number.isInteger(chain) ? chain : null);
  }

  faucet.addEventListener("click", () => {
    void (async () => {
      clearError();
      try {
        const hash = await invoke<string>("wallet_request_airdrop");
        status.textContent = `Faucet claim sent. TxHash: ${hash}`;
        if (pending) {
          await paintFunds(pending.method, pending.value, amount.value, pending.quote.feeAtoms, pending.quote.chainId);
        }
      } catch (error) {
        showError(messageFrom(error));
      }
    })();
  });
  kind.addEventListener("change", () => {
    pending = null;
    reviewBox.hidden = true;
    clearError();
    paintActions();
  });
  action.addEventListener("change", () => {
    pending = null;
    reviewBox.hidden = true;
    clearError();
    paintFields();
  });

  reviewButton.addEventListener("click", () => {
    void (async () => {
      clearError();
      readout.textContent = "";
      status.textContent = "";
      try {
        const current = selectedAction();
        if (!current.sign) return;
        const chainId = chainNumber(shownChainId());
        const target = checkAddress(contract.value, chainId);
        if (!target.ok) throw new Error(target.message);
        const amountAtoms = needs(current.id, "amount") ? atoms(amount.value, "Amount") : null;
        if (amountAtoms != null && amountAtoms <= 0n) throw new Error("Amount must be greater than zero.");
        const valueAtoms = atoms(value.value || "0", "Attached value");
        const gasLimit = atoms(gas.value, "Gas limit");
        const built = buildContractCall({
          contract: selectedKind(),
          method: current.id,
          chainId,
          claimer: recipient.value,
          beneficiary: recipient.value,
          recipient: recipient.value,
          account: owner.value,
          hash: hash.value,
          secret: secret.value,
          id: record.value,
          timelockMs: current.id === "htlc_create" ? atoms(timelock.value, "Timelock") : null,
          lockUntilMs: current.id === "vault_create" ? atoms(timelock.value, "Lock until") : null,
          amountAtoms,
          tokenUnits: current.id === "token_transfer" || current.id === "token_mint" ? amountAtoms : null,
          txId: needs(current.id, "proposal") ? atoms(proposal.value, "Proposal id") : null,
          callDataHex: data.value,
        });
        const quote = await invoke<Quote>("wallet_call_quote");
        const shown = shownChainId();
        if (shown && shown !== quote.chainId) throw new Error(`The node reported chain ${quote.chainId}, not ${shown}.`);
        pending = {
          method: built.method,
          argsHex: bytesToHex(built.args),
          rows: built.details.map((detail) => ({ label: detail.label, value: detail.value })),
          contract: contract.value.trim(),
          value: valueAtoms.toString(),
          gas: gasLimit.toString(),
          quote,
        };
        rows.replaceChildren();
        const view = [
          { label: "Action", value: current.label },
          { label: "Method", value: pending.method },
          { label: "Contract", value: pending.contract },
          ...pending.rows,
          { label: "Args hex", value: pending.argsHex || "none" },
          { label: "Value", value: `${pending.value} atoms` },
          { label: "Fee", value: `${quote.feeDisplay} KVNC (${quote.feeAtoms} atoms)` },
          { label: "Gas limit", value: pending.gas },
          { label: "Nonce", value: quote.nonce },
          { label: "Chain", value: quote.chainId },
        ];
        for (const row of view) {
          const term = document.createElement("dt");
          term.textContent = row.label;
          const body = document.createElement("dd");
          body.textContent = row.value;
          rows.append(term, body);
        }
        mainnet.checked = false;
        mainnetLabel.hidden = quote.chainId !== "1";
        reviewBox.hidden = false;
        signed.value = "";
        await paintFunds(pending.method, pending.value, amountAtoms?.toString() ?? "0", quote.feeAtoms, quote.chainId);
      } catch (error) {
        pending = null;
        reviewBox.hidden = true;
        showError(messageFrom(error));
      }
    })();
  });

  signButton.addEventListener("click", () => {
    void (async () => {
      clearError();
      try {
        if (!pending) throw new Error("Review the call first.");
        if (pending.quote.chainId === "1" && !mainnet.checked) throw new Error("Chain 1 needs an extra confirmation before this wallet will sign.");
        const nonce = Number(pending.quote.nonce);
        const chainId = Number(pending.quote.chainId);
        if (!Number.isSafeInteger(nonce) || !Number.isSafeInteger(chainId)) throw new Error("Nonce or chain id is too large for the desktop command.");
        const raw = await invoke<string>("wallet_sign_call_hd", {
          bip39Passphrase: "",
          accountIndex: 0,
          contract: pending.contract,
          method: pending.method,
          argsHex: pending.argsHex,
          value: pending.value,
          gasLimit: pending.gas,
          fee: pending.quote.feeAtoms,
          nonce,
          chainId,
        });
        await broadcast(raw);
      } catch (error) {
        showError(messageFrom(error));
      }
    })();
  });

  copyButton.addEventListener("click", () => {
    void (async () => {
      clearError();
      try {
        const body = unsignedDoc();
        signed.value = body;
        await navigator.clipboard.writeText(body);
        status.textContent = "Unsigned payload copied. Sign it on the air-gapped phone, then paste the signed JSON below.";
      } catch (error) {
        showError(messageFrom(error));
      }
    })();
  });

  importButton.addEventListener("click", () => {
    void (async () => {
      clearError();
      try {
        const accepted = acceptSigned(signed.value);
        const expected = pending ? Number(pending.quote.chainId) : chainNumber(shownChainId());
        if (expected != null && accepted.doc.chainId !== expected) {
          throw new Error(`This signature is for chain ${accepted.doc.chainId}. The review is chain ${expected}.`);
        }
        await broadcast(accepted.raw);
      } catch (error) {
        showError(messageFrom(error));
      }
    })();
  });

  readButton.addEventListener("click", () => {
    void (async () => {
      clearError();
      readout.textContent = "";
      try {
        const current = selectedAction();
        const chainId = chainNumber(shownChainId());
        const target = checkAddress(contract.value, chainId);
        if (!target.ok) throw new Error(target.message);
        if (current.id === "balance_of") {
          const who = checkAddress(owner.value, chainId);
          if (!who.ok) throw new Error(who.message);
          const key = decodeRecipient(owner.value, chainId);
          const slot = tokenStateSlot();
          const stored = await invoke<string | null>("wallet_storage_at", { address: contract.value.trim(), slot });
          const raw = storageBytes(stored);
          const state = raw ? decodeTokenState(raw) : null;
          if (!state) throw new Error("This contract has no token state yet.");
          const row = state.balances.find((item) => equalBytes(item.account, key));
          readout.textContent = `${state.name} (${state.symbol}) balance_of: ${(row?.amount ?? 0n).toString()} base units. Supply ${state.totalSupply.toString()}.`;
          return;
        }
        const clean = record.value.trim().toLowerCase().replace(/^0x/, "");
        if (!/^[0-9a-f]{64}$/.test(clean)) throw new Error("Record id must be 64 hex characters.");
        const id = hexToBytes(clean);
        const slot = prefixedSlot("kvnc/v1/vault/", id);
        const stored = await invoke<string | null>("wallet_storage_at", { address: contract.value.trim(), slot });
        const vault = storageBytes(stored);
        const decoded = vault ? decodeVault(vault) : null;
        if (!decoded) throw new Error("No vault record at that id.");
        readout.textContent = `Vault status: total ${decoded.total.toString()} atoms, claimed ${decoded.claimed.toString()}, schedule ${decoded.schedule}, cancelled ${decoded.cancelled ? "yes" : "no"}.`;
      } catch (error) {
        showError(messageFrom(error));
      }
    })();
  });

  paintActions();
}

