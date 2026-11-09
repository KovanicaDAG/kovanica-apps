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
