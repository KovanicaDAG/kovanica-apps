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

export async function requestAirdrop(rpcUrl: string, address: string, atoms = "100000000000"): Promise<string> {
  let last = "faucet failed";
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      const result = await post(candidate, "kovanica_requestAirdrop", [address, atoms]);
      if (typeof result === "string" && result.trim()) return result.replace(/^0x/, "");
      const record = asRecord(result);
      const hash = record?.hash ?? record?.txHash ?? record?.transactionHash;
      if (typeof hash === "string" && hash.trim()) return hash.replace(/^0x/, "");
      throw new Error("The faucet did not return a transaction hash");
    } catch (error) {
      last = error instanceof Error ? error.message : last;
    }
  }
  throw new Error(last);
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
  const fromReceipt = await receiptOnce(rpcUrl, hash, "kovanica_getTransactionReceipt");
  if (fromReceipt && fromReceipt.status !== "pending") return fromReceipt;
  const fromTx = await receiptOnce(rpcUrl, hash, "kovanica_getTransaction");
  return fromTx ?? fromReceipt;
}

async function receiptOnce(
  rpcUrl: string,
  hash: string,
  method: "kovanica_getTransactionReceipt" | "kovanica_getTransaction",
): Promise<{ status: "pending" | "confirmed" | "failed"; height: number | null } | null> {
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      const result = await post(candidate, method, [hash]);
      if (result == null) return { status: "pending", height: null };
      const record = asRecord(result) ?? {};
      const statusText = String(record.status ?? record.outcome ?? "").toLowerCase();
      const height = pickNumber(record, ["blockHeight", "block_height", "height", "committedHeight", "round"]);
      if (statusText.includes("fail") || record.success === false) return { status: "failed", height };
      if (statusText.includes("pend")) return { status: "pending", height };
      if (method === "kovanica_getTransaction" && height == null) return { status: "pending", height };
      return { status: "confirmed", height };
    } catch {
      continue;
    }
  }
  return null;
}

export async function indexerLogs(url: string): Promise<unknown> {
  const result = await proxyIndexer({ data: { url } });
  if (!result.ok) throw new Error("indexer did not respond");
  return parseJson(result.body);
}

export async function readContract(rpcUrl: string, address: string, method: string, storageKey: string): Promise<{ call: string; storage: string }> {
  let call = "not returned";
  let storage = "not returned";
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      call = present(await post(candidate, "kovanica_call", [address, method, storageKey]));
      break;
    } catch (error) {
      call = error instanceof Error ? error.message : "call failed";
    }
  }
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      storage = present(await post(candidate, "kovanica_getStorage", [address, storageKey]));
      break;
    } catch (error) {
      storage = error instanceof Error ? error.message : "storage failed";
    }
  }
  return { call, storage };
}

function present(value: unknown): string {
  const amount = quantity(value);
  if (amount != null && (typeof value !== "object" || value === null)) return amount.toString();
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return "unreadable result";
  }
}

async function tryMethod(rpcUrl: string, method: string, params: unknown[]): Promise<string> {
  let last = "not returned";
  for (const candidate of withRpcPath(rpcUrl)) {
    try {
      return present(await post(candidate, method, params));
    } catch (error) {
      last = error instanceof Error ? error.message : last;
    }
  }
  return last;
}

export async function readNodeWatch(rpcUrl: string): Promise<{ dag: string; template: string; header: string }> {
  const [dag, template, header] = await Promise.all([
    tryMethod(rpcUrl, "kovanica_getDagInfo", []),
    tryMethod(rpcUrl, "kovanica_getBlockTemplate", []),
    tryMethod(rpcUrl, "kovanica_getLeaderHeader", []),
  ]);
  return { dag, template, header };
}
