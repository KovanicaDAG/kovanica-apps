import type { TxPreview } from "@/core/session";
import { formatAtoms, formatKvnc } from "@/core/format";

export type ReviewRow = { label: string; value: string; mono?: boolean };

export function reviewRows(preview: TxPreview): ReviewRow[] {
  const rows: ReviewRow[] = [
    { label: "From", value: preview.sender, mono: true },
    { label: preview.kind === "transfer" ? "To" : "Contract", value: preview.counterparty, mono: true },
    { label: "Account", value: preview.pathLabel },
    {
      label: preview.kind === "transfer" ? "Amount" : "Value",
      value: `${formatKvnc(BigInt(preview.kind === "transfer" ? preview.amountAtoms : preview.valueAtoms))} KVNC (${formatAtoms(BigInt(preview.kind === "transfer" ? preview.amountAtoms : preview.valueAtoms))} atoms)`,
    },
    { label: "Fee", value: `${formatKvnc(BigInt(preview.feeAtoms))} KVNC (${formatAtoms(BigInt(preview.feeAtoms))} atoms)` },
    { label: "Nonce", value: preview.nonce },
    { label: "Chain", value: `${preview.chainId} · ${preview.networkName}` },
  ];
  if (preview.kind === "call") {
    rows.splice(3, 0, { label: "Method", value: preview.method });
    rows.push({ label: "Gas limit", value: preview.gasLimit });
  }
  return rows;
}
