/** One event the node returned with a receipt or from `kovanica_getLogs`. */
export type ChainLog = {
  address: string;
  topics: string[];
  data: string;
  height: string | null;
  hash: string | null;
};

/** Execution result. `code` is `0x1` for success and `0x0` for a revert. */
export type ExecutionReceipt = {
  status: "pending" | "success" | "revert";
  code: "0x1" | "0x0" | null;
  gasUsed: string | null;
  height: string | null;
  hash: string | null;
  error: string | null;
  logs: ChainLog[];
};

export type LiveProbe = {
  url: string;
  reachable: boolean;
  chainId: number | null;
  error: string | null;
};

const CHAIN5 = ["http://127.0.0.1:8545", "http://127.0.0.1:8545/rpc"];

export function chain5Candidates(extra?: string): string[] {
  const urls = [extra, ...CHAIN5].filter((item): item is string => Boolean(item && item.trim()));
  return [...new Set(urls)];
}

export function decodeReceipt(value: unknown): ExecutionReceipt {
  if (value == null) return pending();
  if (Array.isArray(value)) return { ...pending(), logs: decodeLogs(value) };
  if (typeof value !== "object") return pending();
  const record = value as Record<string, unknown>;
  const statusText = text(record.status ?? record.outcome).toLowerCase();
  const reverted =
    statusText === "0x0" ||
    statusText === "0" ||
    statusText.includes("fail") ||
    statusText.includes("revert") ||
    record.success === false;
  const mined = pick(record, ["blockNumber", "blockHeight", "block_height", "height", "round"]) != null;
  const waiting = !reverted && (statusText.includes("pend") || (statusText === "" && !mined));
  const code = reverted ? "0x0" : waiting ? null : "0x1";
  return {
    status: reverted ? "revert" : waiting ? "pending" : "success",
    code,
    gasUsed: quantity(pick(record, ["gasUsed", "gas_used"])),
    height: quantity(pick(record, ["blockNumber", "blockHeight", "block_height", "height", "round"])),
    hash: stripHex(text(pick(record, ["transactionHash", "txHash", "hash"]))),
    error: reverted ? text(pick(record, ["revertReason", "revert_reason", "error", "reason", "message"])) || "The contract reverted." : null,
    logs: decodeLogs(record.logs ?? record.events),
  };
}

export function decodeLogs(value: unknown): ChainLog[] {
  const list = Array.isArray(value)
    ? value
    : value && typeof value === "object" && Array.isArray((value as { logs?: unknown }).logs)
      ? (value as { logs: unknown[] }).logs
      : [];
  return list.flatMap((item) => {
    if (!item || typeof item !== "object") return [];
    const record = item as Record<string, unknown>;
    const topics = Array.isArray(record.topics) ? record.topics.map((topic) => String(topic)) : [];
    return [
      {
        address: text(record.address),
        topics,
        data: text(record.data),
        height: quantity(pick(record, ["blockNumber", "blockHeight", "height"])),
        hash: stripHex(text(pick(record, ["transactionHash", "hash"]))),
      },
    ];
  });
}

export function formatReceipt(view: ExecutionReceipt): string {
  const head = view.status === "success" ? `status ${view.code ?? "0x1"} Success` : view.status === "revert" ? `status ${view.code ?? "0x0"} Revert` : "status Pending";
  const lines = [head];
  if (view.gasUsed) lines.push(`gasUsed ${view.gasUsed}`);
  if (view.height) lines.push(`block ${view.height}`);
  if (view.hash) lines.push(`tx ${view.hash}`);
  if (view.error) lines.push(view.error);
  if (view.logs.length) {
    lines.push(`logs ${view.logs.length}`);
    for (const log of view.logs.slice(0, 3)) lines.push(`${log.topics[0] ?? "event"} @ ${log.height ?? "?"}`);
  }
  return lines.join("\n");
}

/** Ask a Chain 5 node for its sync status. A down node is unreachable, never a fake receipt. */
export async function probeChain5(extra?: string, timeoutMs = 2500): Promise<LiveProbe> {
  let last = "unreachable";
  let url = "";
  for (const candidate of chain5Candidates(extra)) {
    url = candidate.endsWith("/rpc") ? candidate : `${candidate.replace(/\/$/, "")}/rpc`;
    try {
      const response = await fetch(url, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "kovanica_syncStatus", params: [] }),
        signal: AbortSignal.timeout(timeoutMs),
      });
      if (!response.ok) {
        last = `HTTP ${response.status}`;
        continue;
      }
      const body = (await response.json()) as { result?: unknown; error?: unknown };
      if (body.error) {
        last = typeof body.error === "string" ? body.error : "rpc error";
        continue;
      }
      const chainId = chainIdOf(body.result);
      if (chainId !== 5) {
        return { url, reachable: true, chainId, error: chainId == null ? "no chain id" : `node reported chain ${chainId}, not 5` };
      }
      return { url, reachable: true, chainId, error: null };
    } catch (error) {
      last = error instanceof Error ? error.message : "unreachable";
    }
  }
  return { url, reachable: false, chainId: null, error: last };
}

export async function postRpc(url: string, method: string, params: unknown[], timeoutMs = 2500): Promise<unknown> {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const body = (await response.json()) as { result?: unknown; error?: { message?: string } | string };
  if (body.error) throw new Error(typeof body.error === "string" ? body.error : body.error.message || "rpc error");
  return body.result ?? null;
}

function pending(): ExecutionReceipt {
  return { status: "pending", code: null, gasUsed: null, height: null, hash: null, error: null, logs: [] };
}

function chainIdOf(value: unknown): number | null {
  if (!value || typeof value !== "object") return quantityNumber(value);
  const record = value as Record<string, unknown>;
  return quantityNumber(pick(record, ["chainId", "chain_id", "chainID"])) ?? chainIdOf(record.network);
}

function quantityNumber(value: unknown): number | null {
  const text = quantity(value);
  if (!text) return null;
  const parsed = Number(text);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function quantity(value: unknown): string | null {
  if (typeof value === "number" && Number.isFinite(value)) return Math.trunc(value).toString();
  if (typeof value === "bigint") return value.toString();
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("0x") || trimmed.startsWith("0X")) {
    try {
      return BigInt(trimmed).toString();
    } catch {
      return null;
    }
  }
  return /^\d+$/.test(trimmed) ? trimmed.replace(/^0+(?=\d)/, "") : null;
}

function pick(record: Record<string, unknown>, keys: string[]): unknown {
  for (const key of keys) if (record[key] != null && record[key] !== "") return record[key];
  return null;
}

function text(value: unknown): string {
  return value == null ? "" : String(value).trim();
}

function stripHex(value: string): string | null {
  const clean = value.replace(/^0x/i, "");
  return clean || null;
}
