import { useEffect, useState } from "react";
import { FAUCET_ATOMS, cooldownRemaining, faucetChain, rememberClaim } from "@/core/faucet";
import { faucetQuick } from "@/core/history-row";
import { readAccount, requestAirdrop } from "@/services/rpc";
import { useActivity } from "@/stores/activity";
import { useNetwork } from "@/stores/network";
import { toast } from "sonner";

export function FaucetButton({ address, onlyHybrid = false }: { address: string; onlyHybrid?: boolean }) {
  const net = useNetwork();
  const chainId = net.probedChainId;
  const [left, setLeft] = useState(0);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const tick = () => setLeft(cooldownRemaining(chainId, address, Date.now()));
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [chainId, address]);

  if (onlyHybrid ? !faucetQuick(chainId) : !faucetChain(chainId)) return null;
  const seconds = Math.ceil(left / 1000);

  return (
    <button
      type="button"
      className="min-h-11 w-fit rounded-full border border-copper px-4 text-sm text-copper disabled:opacity-40"
      disabled={busy || left > 0 || !net.online}
      onClick={() => {
        setBusy(true);
        void requestAirdrop(net.rpcUrl, address, FAUCET_ATOMS)
          .then((hash) => {
            rememberClaim(chainId, address, Date.now());
            setLeft(cooldownRemaining(chainId, address, Date.now()));
            useActivity.getState().push({
              hash,
              status: "pending",
              direction: "received",
              amountAtoms: FAUCET_ATOMS,
              feeAtoms: "0",
              counterparty: "faucet",
              chainId: chainId ?? 0,
              createdAt: Date.now(),
              height: null,
              note: "faucet",
            });
            toast("Faucet claim sent");
            return readAccount(net.rpcUrl, address);
          })
          .then((state) => {
            if (state) useNetwork.getState().setAccountState(state.balance, state.nonce, state.fee);
          })
          .catch((error: unknown) => toast(error instanceof Error ? error.message : "The faucet did not respond"))
          .finally(() => setBusy(false));
      }}
    >
      {busy ? "Requesting…" : left > 0 ? `Claim again in ${seconds}s` : onlyHybrid ? "Request Testnet Faucet" : "Request Testnet KVNC"}
    </button>
  );
}
