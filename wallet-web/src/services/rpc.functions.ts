import { createServerFn } from "@tanstack/react-start";
import { assertRpcUrl } from "@/lib/networks";

const RPC_METHODS = new Set([
  "kovanica_syncStatus",
  "kovanica_getBalance",
  "kovanica_getNonce",
  "kovanica_estimateFee",
  "kovanica_sendRawTransaction",
  "kovanica_getTransactionReceipt",
  "kovanica_getTransaction",
  "kovanica_blockNumber",
  "kovanica_getStorage",
  "kovanica_getStorageAt",
  "kovanica_call",
  "kovanica_getDagInfo",
  "kovanica_getBlockTemplate",
  "kovanica_getBlockByRound",
  "kovanica_getLeaderHeader",
  "kovanica_requestAirdrop",
]);

type RpcInput = { rpcUrl: string; method: string; params: unknown[] };
type HttpInput = { url: string };

function parseRpc(input: unknown): RpcInput {
  if (!input || typeof input !== "object") throw new Error("missing request");
  const value = input as Record<string, unknown>;
  if (typeof value.rpcUrl !== "string" || typeof value.method !== "string" || !Array.isArray(value.params)) {
    throw new Error("malformed RPC request");
  }
  if (!RPC_METHODS.has(value.method)) throw new Error("RPC method is not used by the wallet");
  if (value.params.length > 8) throw new Error("too many RPC params");
  assertRpcUrl(value.rpcUrl);
  return { rpcUrl: value.rpcUrl, method: value.method, params: value.params };
}

function parseHttp(input: unknown): HttpInput {
  if (!input || typeof input !== "object") throw new Error("missing request");
  const url = (input as Record<string, unknown>).url;
  if (typeof url !== "string") throw new Error("missing url");
  assertRpcUrl(url);
  return { url };
}

export const proxyRpc = createServerFn({ method: "POST" })
  .validator(parseRpc)
  .handler(async ({ data }) => {
    const started = Date.now();
    const response = await fetch(data.rpcUrl, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: data.method, params: data.params }),
      signal: AbortSignal.timeout(8000),
    });
    const text = await response.text();
    return { ok: response.ok, status: response.status, body: text.slice(0, 20_000), ms: Date.now() - started };
  });

export const proxyHealth = createServerFn({ method: "POST" })
  .validator(parseHttp)
  .handler(async ({ data }) => {
    const base = new URL(data.url);
    base.pathname = "/health";
    base.search = "";
    const started = Date.now();
    const response = await fetch(base, { signal: AbortSignal.timeout(8000) });
    const text = await response.text();
    return { ok: response.ok, status: response.status, body: text.slice(0, 4000), ms: Date.now() - started };
  });

export const proxyIndexer = createServerFn({ method: "POST" })
  .validator(parseHttp)
  .handler(async ({ data }) => {
    const response = await fetch(data.url, {
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(8000),
    });
    const text = await response.text();
    return { ok: response.ok, status: response.status, body: text.slice(0, 40_000) };
  });
