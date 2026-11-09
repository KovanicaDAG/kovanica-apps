import { useState } from "react";
import { backupJson, restoreBackup } from "@/services/backup";
import { revealWithPassword } from "@/services/wallet";
import { LOCK_CHOICES, useSettings } from "@/stores/settings";

export function SettingsDrawer({ onClose }: { onClose: () => void }) {
  const minutes = useSettings((s) => s.lockMinutes);
  const setMinutes = useSettings((s) => s.setLockMinutes);
  const [password, setPassword] = useState("");
  const [phrase, setPhrase] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function download() {
    const blob = new Blob([backupJson()], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "kovanica-wallet-backup.json";
    link.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div className="fixed inset-0 z-30 flex justify-end bg-ink/70" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <button type="button" className="flex-1" aria-label="Close settings" onClick={onClose} />
      <div className="h-full w-full max-w-md overflow-y-auto border-l border-line bg-surface p-5">
        <div className="flex items-center justify-between">
          <h2 id="settings-title" className="text-2xl font-semibold text-paper">Settings</h2>
          <button type="button" className="min-h-11 px-3 text-muted" onClick={onClose}>Close</button>
        </div>
        <p className="mt-2 text-sm text-muted">The phrase is shown only after the password check, and it is not saved.</p>
        <form
          className="mt-5 grid gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            setError(null);
            setBusy(true);
            void revealWithPassword(password)
              .then((next) => setPhrase(next))
              .catch((err: unknown) => setError(err instanceof Error ? err.message : "Could not reveal the phrase"))
              .finally(() => setBusy(false));
          }}
        >
          <label className="grid gap-1 text-sm text-muted">
            Password
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
          </label>
          <button type="submit" className="min-h-11 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={busy}>
            {busy ? "Checking…" : "Show recovery phrase"}
          </button>
        </form>
        {phrase ? (
          <div className="mt-4">
            <p className="mono text-sm break-words text-paper">{phrase}</p>
            <button type="button" className="mt-3 min-h-11 text-sm text-copper" onClick={() => setPhrase(null)}>Hide phrase</button>
          </div>
        ) : null}
        {error ? <p className="mt-3 text-sm text-clay">{error}</p> : null}
        <h3 className="mt-8 text-sm tracking-wide text-muted uppercase">Lock timeout</h3>
        <div className="mt-3 grid grid-cols-4 gap-2">
          {LOCK_CHOICES.map((choice) => (
            <button key={choice} type="button" className={`min-h-11 rounded-full border text-sm ${minutes === choice ? "border-copper text-copper" : "border-line text-muted"}`} onClick={() => setMinutes(choice)}>
              {choice}m
            </button>
          ))}
        </div>
        <h3 className="mt-8 text-sm tracking-wide text-muted uppercase">Backup</h3>
        <p className="mt-2 text-sm text-muted">Exports the encrypted keystore from this browser, plus contacts and the watchlist. The phrase is not in the file.</p>
        <button type="button" className="mt-3 min-h-11 rounded-full border border-line px-4 text-paper" onClick={download}>Export backup</button>
        <label className="mt-3 grid gap-1 text-sm text-muted">
          Import backup
          <input
            className="text-sm text-paper"
            type="file"
            accept="application/json,.json"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (!file) return;
              void file.text().then((text) => {
                setError(null);
                try {
                  restoreBackup(text);
                  setPhrase(null);
                  setError(null);
                  onClose();
                } catch (err) {
                  setError(err instanceof Error ? err.message : "Could not import the backup");
                }
              });
            }}
          />
        </label>
      </div>
    </div>
  );
}
