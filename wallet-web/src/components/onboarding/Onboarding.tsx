import { useMemo, useState } from "react";
import { generatePhrase, isValidMnemonic, mnemonicWordCount, normalizeMnemonic, suggestWords } from "@/core/hd";
import { createDesktopWallet, createHdWallet, importWallet, isDesktopShell, unlock } from "@/services/wallet";
import { useSession } from "@/stores/session";

type Mode = "choose" | "create" | "import" | "unlock";

export function Onboarding({ mode }: { mode: Mode }) {
  const usesPass = useSession((s) => s.usesBip39Passphrase);
  const kind = useSession((s) => s.kind);
  const address = useSession((s) => s.accounts[0]?.address ?? "");
  const [screen, setScreen] = useState<Mode>(mode);
  const [words, setWords] = useState<12 | 24>(24);
  const [phrase, setPhrase] = useState("");
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const [bip39, setBip39] = useState("");
  const [importMode, setImportMode] = useState<"hd" | "legacy" | null>(null);
  const [check, setCheck] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [suggest, setSuggest] = useState<string[]>([]);

  const list = useMemo(() => normalizeMnemonic(phrase).split(" ").filter(Boolean), [phrase]);
  const quizIndex = list.length > 2 ? 3 : 1;
  const valid = isValidMnemonic(phrase);
  const count = mnemonicWordCount(phrase);

  const desktop = isDesktopShell();

  async function sealCreate() {
    setError(null);
    if (password.length < 8) return setError("Password must be at least 8 characters.");
    if (password !== again) return setError("Passwords do not match.");
    if (desktop) {
      setBusy(true);
      try {
        await createDesktopWallet(words, password);
        setPassword("");
        setAgain("");
      } catch (err) {
        setError(err instanceof Error ? err.message : "Could not create the wallet");
      } finally {
        setBusy(false);
      }
      return;
    }
    if (!valid) return setError("The phrase checksum does not match.");
    if (list[quizIndex - 1] !== check.trim().toLowerCase()) return setError(`Word ${quizIndex} does not match.`);
    setBusy(true);
    try {
      await createHdWallet(phrase, password);
      setPhrase("");
      setPassword("");
      setAgain("");
      setCheck("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not seal the wallet");
    } finally {
      setBusy(false);
    }
  }

  async function sealImport() {
    setError(null);
    if (!valid) return setError("The phrase checksum does not match.");
    if (count === 24 && !importMode) return setError("Choose hierarchical or legacy. They are not the same key.");
    if (password.length < 8) return setError("Password must be at least 8 characters.");
    setBusy(true);
    try {
      await importWallet({
        phrase,
        mode: count === 12 ? "hd" : (importMode ?? "hd"),
        password,
        bip39Passphrase: importMode === "legacy" ? "" : bip39,
      });
      setPhrase("");
      setPassword("");
      setBip39("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not import");
    } finally {
      setBusy(false);
    }
  }

  async function doUnlock() {
    setError(null);
    setBusy(true);
    try {
      await unlock(password, bip39);
      setPassword("");
      setBip39("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unlock failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="mx-auto flex min-h-screen w-full max-w-lg flex-col justify-center px-5 py-10">
      <p className="text-xs tracking-[0.22em] text-copper uppercase">Kovanica</p>
      <h1 className="mt-2 text-4xl font-semibold text-paper">Wallet</h1>
      <p className="mt-3 text-muted">KVNC stays on this device. Signing happens only after you review the transaction.</p>

      {screen === "choose" ? (
        <div className="mt-8 grid gap-3">
          <button type="button" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink" onClick={() => { if (!desktop) setPhrase(generatePhrase(24)); setScreen("create"); }}>
            Create a new wallet
          </button>
          <button type="button" className="min-h-12 rounded-full border border-line text-paper" onClick={() => setScreen("import")}>
            Import a phrase
          </button>
        </div>
      ) : null}

      {screen === "unlock" ? (
        <form className="mt-8 grid gap-3" onSubmit={(e) => { e.preventDefault(); void doUnlock(); }}>
          <p className="mono text-sm break-all text-paper">{address}</p>
          {kind === "legacy" ? <p className="text-sm text-copper">Legacy account</p> : null}
          <label className="grid gap-1 text-sm text-muted">
            Password
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} />
          </label>
          {usesPass ? (
            <label className="grid gap-1 text-sm text-muted">
              BIP-39 passphrase
              <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" value={bip39} onChange={(e) => setBip39(e.target.value)} />
            </label>
          ) : null}
          <button type="submit" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={busy}>
            {busy ? "Unlocking…" : "Unlock"}
          </button>
        </form>
      ) : null}

      {screen === "create" ? (
        <div className="mt-8 grid gap-4">
          <div className="flex gap-2">
            {([12, 24] as const).map((n) => (
              <button key={n} type="button" className={`min-h-11 flex-1 rounded-full border ${words === n ? "border-copper text-copper" : "border-line text-muted"}`} onClick={() => { setWords(n); if (!desktop) { setPhrase(generatePhrase(n)); setCheck(""); } }}>
                {n} words
              </button>
            ))}
          </div>
          <ol className={`grid grid-cols-2 gap-2 sm:grid-cols-3 ${desktop ? "hidden" : ""}`}>
            {normalizeMnemonic(phrase).split(" ").filter(Boolean).map((word, i) => (
              <li key={`${word}-${i}`} className="rounded-xl border border-line bg-surface px-3 py-2 text-sm">
                <span className="mr-2 text-muted">{i + 1}</span>
                {word}
              </li>
            ))}
          </ol>
          {desktop ? <p className="text-sm text-muted">The desktop signer creates the phrase. It is shown once, then cleared when the wallet locks.</p> : null}
          {desktop ? null : (
          <label className="grid gap-1 text-sm text-muted">
            Confirm word {quizIndex}
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" value={check} onChange={(e) => setCheck(e.target.value)} autoComplete="off" />
          </label>
          )}
          <label className="grid gap-1 text-sm text-muted">
            Password
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" />
          </label>
          <label className="grid gap-1 text-sm text-muted">
            Repeat password
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" value={again} onChange={(e) => setAgain(e.target.value)} autoComplete="new-password" />
          </label>
          <button type="button" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={busy} onClick={() => void sealCreate()}>
            {busy ? "Sealing keystore…" : "Save wallet"}
          </button>
          <button type="button" className="min-h-11 text-muted" onClick={() => setScreen("choose")}>Back</button>
        </div>
      ) : null}

      {screen === "import" ? (
        <form className="mt-8 grid gap-3" onSubmit={(e) => { e.preventDefault(); void sealImport(); }}>
          <label className="grid gap-1 text-sm text-muted">
            Recovery phrase
            <textarea
              className="min-h-28 rounded-2xl border border-line bg-ink px-4 py-3 text-paper"
              value={phrase}
              onChange={(e) => {
                setPhrase(e.target.value);
                const last = e.target.value.split(/\s+/).at(-1) ?? "";
                setSuggest(last ? suggestWords(last) : []);
              }}
              autoComplete="off"
              spellCheck={false}
            />
          </label>
          {suggest.length > 0 ? (
            <div className="flex flex-wrap gap-2">
              {suggest.map((word) => (
                <button
                  key={word}
                  type="button"
                  className="rounded-full border border-line px-3 py-1 text-sm text-paper"
                  onClick={() => {
                    const parts = phrase.trim().split(/\s+/);
                    parts[parts.length - 1] = word;
                    setPhrase(`${parts.join(" ")} `);
                    setSuggest([]);
                  }}
                >
                  {word}
                </button>
              ))}
            </div>
          ) : null}
          <p className={valid ? "text-sm text-sage" : "text-sm text-muted"}>{phrase.trim() ? (valid ? `Checksum matches · ${count} words` : "Checksum does not match yet") : "English BIP-39 words only"}</p>
          {count === 24 && valid ? (
            <fieldset className="grid gap-2">
              <legend className="text-sm text-muted">This 24-word phrase is</legend>
              <button type="button" className={`min-h-11 rounded-2xl border px-4 text-left ${importMode === "hd" ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => setImportMode("hd")}>
                Hierarchical · m/44'/3007'/…
              </button>
              <button type="button" className={`min-h-11 rounded-2xl border px-4 text-left ${importMode === "legacy" ? "border-copper text-paper" : "border-line text-muted"}`} onClick={() => setImportMode("legacy")}>
                Legacy account · the words are the key
              </button>
            </fieldset>
          ) : null}
          {importMode !== "legacy" ? (
            <label className="grid gap-1 text-sm text-muted">
              BIP-39 passphrase, if you used one
              <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" value={bip39} onChange={(e) => setBip39(e.target.value)} />
            </label>
          ) : null}
          <label className="grid gap-1 text-sm text-muted">
            Keystore password
            <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" />
          </label>
          <button type="submit" className="min-h-12 rounded-full bg-copper font-medium text-copper-ink disabled:opacity-40" disabled={busy}>
            {busy ? "Sealing keystore…" : "Import"}
          </button>
          <button type="button" className="min-h-11 text-muted" onClick={() => setScreen("choose")}>Back</button>
        </form>
      ) : null}
      {error ? <p className="mt-4 text-sm text-clay">{error}</p> : null}
    </section>
  );
}
