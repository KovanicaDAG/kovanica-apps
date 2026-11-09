import { proxyHealth, proxyIndexer, proxyRpc } from "./rpc.functions";

export type SyncProbe = {
  chainId: number | null;
  height: number | null;
  peerCount: number | null;
  latencyMs: number | null;
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
}

function pickNumber(source: Record<string, unknown>, keys: string[]): number | null {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "string" && /^-?\d+$/.test(value)) return Number(value);
  }
  return null;
}

function parseJson(body: string): unknown {
  try {
    return JSON.parse(body);
  } catch {
    return null;
  }
}

async function post(rpcUrl: string, method: string, params: unknown[]): Promise<unknown> {
  const result = await proxyRpc({ data: { rpcUrl, method, params } });
  if (!result.ok) throw new Error(`node returned ${result.status}`);
  const parsed = asRecord(parseJson(result.body));
  if (!parsed) throw new Error("node did not return JSON");
  if (parsed.error) {
    const err = asRecord(parsed.error);
    throw new Error(typeof err?.message === "string" ? err.message : "RPC error");
  }
  return parsed.result ?? parsed;
}

function withRpcPath(url: string): string[] {
  const primary = url.replace(/\/$/, "");
  const candidates = [primary];
  if (!primary.endsWith("/rpc")) candidates.push(`${primary}/rpc`);
  return candidates;
}

export async function probeNode(rpcUrl: string): Promise<SyncProbe & { latencyMs: number | null; online: boolean; error: string | null }> {
  let last = "Node did not respond";
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      const health = await proxyHealth({ data: { url: candidate } }).catch(() => null);
      const started = Date.now();
      const result = await post(candidate, "kovanica_syncStatus", []);
      const record = asRecord(result) ?? {};
      const chainId = pickNumber(record, ["chainId", "chain_id", "chainID"]);
      const height = pickNumber(record, ["committedHeight", "committed_height"]);
      const peerCount = pickNumber(record, ["peerCount", "peer_count"]);
      return {
        chainId,
        height,
        peerCount,
        latencyMs: health?.ms ?? Date.now() - started,
        online: true,
        error: chainId == null ? "Node responded, but it did not report a chain id." : null,
      };
    } catch (error) {
      last = error instanceof Error ? error.message : last;
    }
  }
  return { chainId: null, height: null, peerCount: null, latencyMs: null, online: false, error: last };
}

function quantity(value: unknown): bigint | null {
  if (typeof value === "bigint") return value;
  if (typeof value === "number" && Number.isSafeInteger(value)) return BigInt(value);
  if (typeof value === "string") {
    const text = value.trim();
    if (/^\d+$/.test(text)) return BigInt(text);
    if (/^0x[0-9a-f]+$/i.test(text)) return BigInt(text);
  }
  const record = asRecord(value);
  if (!record) return null;
  return quantity(record.balance ?? record.atoms ?? record.nonce ?? record.fee ?? record.result);
}

export async function readAccount(rpcUrl: string, address: string): Promise<{ balance: bigint | null; nonce: bigint | null; fee: bigint | null }> {
  let balance: bigint | null = null;
  let nonce: bigint | null = null;
  let fee: bigint | null = null;
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      balance = quantity(await post(candidate, "kovanica_getBalance", [address]));
      nonce = quantity(await post(candidate, "kovanica_getNonce", [address]));
      try {
        fee = quantity(await post(candidate, "kovanica_estimateFee", [{ to: address }]));
      } catch {
        fee = quantity(await post(candidate, "kovanica_estimateFee", []));
      }
      return { balance, nonce, fee };
    } catch {
      continue;
    }
  }
  return { balance, nonce, fee };
}

export async function broadcast(rpcUrl: string, rawHex: string): Promise<string> {
  let last = "broadcast failed";
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      const result = await post(candidate, "kovanica_sendRawTransaction", [rawHex]);
      if (typeof result === "string") return result.replace(/^0x/, "");
      const record = asRecord(result);
      const hash = record?.hash ?? record?.txHash ?? record?.transactionHash;
      if (typeof hash === "string") return hash.replace(/^0x/, "");
      return rawHex.slice(0, 64);
    } catch (error) {
      last = error instanceof Error ? error.message : last;
    }
  }
  throw new Error(last);
}

export async function receipt(rpcUrl: string, hash: string): Promise<{ status: "pending" | "confirmed" | "failed"; height: number | null } | null> {
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      const result = await post(candidate, "kovanica_getTransactionReceipt", [hash]);
      if (result == null) return { status: "pending", height: null };
      const record = asRecord(result);
      if (!record) return { status: "pending", height: null };
      const height = pickNumber(record, ["height", "blockHeight", "block_height", "committedHeight"]);
      const statusText = String(record.status ?? record.outcome ?? "").toLowerCase();
      if (statusText.includes("fail") || record.success === false) return { status: "failed", height };
      if (height != null || statusText.includes("confirm") || record.success === true) {
        return { status: "confirmed", height };
      }
      return { status: "pending", height };
    } catch {
      continue;
    }
  }
  return null;
}

export async function indexerLogs(url: string): Promise<unknown> {
  const result = await proxyIndexer({ data: { url } });
  if (!result.ok) throw new Error(`indexer returned ${result.status}`);
  return parseJson(result.body);
}
