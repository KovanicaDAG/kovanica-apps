import { useCallback, useEffect, useState } from "react";

import {
  deriveCommsId,
  createRpcClient,
  fetchChainHead,
  LOCAL_RPC_URL,
  NETWORK_PRESETS,
} from "@kovanica/comms";
import type { ChainHead } from "@kovanica/comms";
import { encodeAddress, keyFromHexSeed } from "@kovanica/signer";

/**
 * M0 scaffold screen: derive the comms identity from a wallet key and read the
 * chain head over RPC. Deliberately minimal — no group/messaging UI yet.
 */
export function App() {
  const [seedHex, setSeedHex] = useState("");
  const [address, setAddress] = useState<string | null>(null);
  const [commsId, setCommsId] = useState<string | null>(null);
  const [identityError, setIdentityError] = useState<string | null>(null);

  const [rpcUrl, setRpcUrl] = useState(LOCAL_RPC_URL);
  const [head, setHead] = useState<ChainHead | null>(null);
  const [headError, setHeadError] = useState<string | null>(null);
  const [headLoading, setHeadLoading] = useState(false);

  /** The preset whose endpoint matches the field, or null for a custom URL. */
  const activePreset = NETWORK_PRESETS.find((preset) => preset.rpcUrl === rpcUrl.trim()) ?? null;

  const loadIdentity = useCallback(async () => {
    setIdentityError(null);
    setAddress(null);
    setCommsId(null);
    try {
      // 32-byte hex seed, with or without `0x` (same input as `kovanica import-key`).
      const key = await keyFromHexSeed(seedHex);
      setAddress(encodeAddress(key.publicKey));
      setCommsId(deriveCommsId(key.publicKey));
    } catch (error) {
      setIdentityError(error instanceof Error ? error.message : String(error));
    }
  }, [seedHex]);

  const refreshHead = useCallback(async () => {
    setHeadLoading(true);
    setHeadError(null);
    try {
      const client = createRpcClient(rpcUrl);
      setHead(await fetchChainHead(client));
    } catch (error) {
      setHead(null);
      setHeadError(error instanceof Error ? error.message : String(error));
    } finally {
      setHeadLoading(false);
    }
  }, [rpcUrl]);

  // Best-effort head read on first render; a node may not be running.
  useEffect(() => {
    void refreshHead();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <main className="app">
      <h1>Kovanica Comms</h1>
      <p className="muted">M0 scaffold — identity + chain head only.</p>

      <section className="panel">
        <h2>Wallet identity</h2>
        <label htmlFor="seed">Wallet seed (32-byte hex)</label>
        <input
          id="seed"
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder="0x… or bare hex"
          value={seedHex}
          onChange={(event) => setSeedHex(event.target.value)}
        />
        <button type="button" onClick={() => void loadIdentity()} disabled={seedHex.trim() === ""}>
          Derive comms id
        </button>

        {identityError && <p className="error">{identityError}</p>}

        {commsId && (
          <dl className="readout">
            <dt>Address</dt>
            <dd className="mono">{address}</dd>
            <dt>Comms id</dt>
            <dd className="mono">{commsId}</dd>
          </dl>
        )}
      </section>

      <section className="panel">
        <h2>Chain head</h2>
        <div className="network-switch" role="group" aria-label="Network">
          {NETWORK_PRESETS.map((preset) => (
            <button
              key={preset.id}
              type="button"
              className={activePreset?.id === preset.id ? "network-btn active" : "network-btn"}
              aria-pressed={activePreset?.id === preset.id}
              onClick={() => setRpcUrl(preset.rpcUrl)}
            >
              {preset.label}
            </button>
          ))}
          <span className="muted">{activePreset ? `Chain ${activePreset.chainId}` : "Custom endpoint"}</span>
        </div>
        <label htmlFor="rpc">JSON-RPC URL</label>
        <input
          id="rpc"
          type="text"
          spellCheck={false}
          value={rpcUrl}
          onChange={(event) => setRpcUrl(event.target.value)}
        />
        <button type="button" onClick={() => void refreshHead()} disabled={headLoading}>
          {headLoading ? "Reading…" : "Refresh"}
        </button>

        {headError && <p className="error">{headError}</p>}

        {head && (
          <dl className="readout">
            <dt>Committed height</dt>
            <dd className="mono">{head.height.toString()}</dd>
            <dt>Height (hex)</dt>
            <dd className="mono">{head.heightHex}</dd>
          </dl>
        )}
      </section>
    </main>
  );
}
