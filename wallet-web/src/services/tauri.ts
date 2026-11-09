type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type Unlisten = () => void;

export function isDesktopShell(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktopShell()) throw new Error("desktop signer is not available");
  const mod = await import("@tauri-apps/api/core");
  const invoke = mod.invoke as Invoke;
  return invoke<T>(cmd, args);
}

export async function onWalletLock(handler: () => void): Promise<Unlisten> {
  if (!isDesktopShell()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const stops = await Promise.all([
    listen("wallet-locked", () => handler()),
    listen("wallet-lock-warning", () => handler()),
  ]);
  return () => {
    for (const stop of stops) stop();
  };
}

export type CallSignArgs = {
  contract: string;
  method: string;
  argsHex: string;
  value: string;
  gasLimit: string;
  fee: string;
  nonce: number;
  chainId: number;
};

export function signCallHd(args: CallSignArgs & { bip39Passphrase: string; accountIndex: number }): Promise<string> {
  return tauriInvoke<string>("wallet_sign_call_hd", args);
}

export function signCall(args: CallSignArgs): Promise<string> {
  return tauriInvoke<string>("wallet_sign_call", args);
}

export async function revealMnemonic(passphrase: string): Promise<string> {
  const result = await tauriInvoke<string | { mnemonic?: string }>("wallet_reveal", { passphrase });
  if (typeof result === "string" && result.trim()) return result;
  if (result && typeof result === "object" && typeof result.mnemonic === "string" && result.mnemonic.trim()) return result.mnemonic;
  throw new Error("desktop signer did not return a phrase");
}

export function touchSession(): Promise<void> {
  return tauriInvoke<void>("wallet_touch");
}
