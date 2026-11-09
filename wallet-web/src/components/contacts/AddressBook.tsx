import { useState } from "react";
import { presetById } from "@/lib/networks";
import { loadContacts, removeContact, saveContact, type Contact } from "@/stores/contacts";
import { useNetwork } from "@/stores/network";

export function AddressBook() {
  const net = useNetwork();
  const chainId = net.probedChainId ?? presetById(net.presetId).expectedChainId;
  const [contacts, setContacts] = useState<Contact[]>(() => loadContacts());
  const [label, setLabel] = useState("");
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);

  return (
    <div className="grid gap-3 rounded-card border border-line bg-surface p-4">
      <h2 className="text-lg font-semibold">Address book</h2>
      <form
        className="grid gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          setError(null);
          try {
            saveContact(label, address, chainId);
            setContacts(loadContacts());
            setLabel("");
            setAddress("");
          } catch (err) {
            setError(err instanceof Error ? err.message : "Could not save the contact");
          }
        }}
      >
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Label" />
        <input className="min-h-11 rounded-2xl border border-line bg-ink px-4 font-mono text-sm text-paper" value={address} onChange={(e) => setAddress(e.target.value)} placeholder="kvnc…dag" autoCapitalize="none" spellCheck={false} />
        <button type="submit" className="min-h-11 rounded-full border border-line text-paper">Save contact</button>
      </form>
      {error ? <p className="text-sm text-clay">{error}</p> : null}
      {contacts.map((contact) => (
        <div key={contact.id} className="flex items-start justify-between gap-3">
          <div>
            <p className="text-sm text-paper">{contact.label}</p>
            <p className="mono text-xs break-all text-muted">{contact.address}</p>
          </div>
          <button type="button" className="min-h-11 text-sm text-clay" onClick={() => setContacts(removeContact(contact.id))}>
            Remove
          </button>
        </div>
      ))}
    </div>
  );
}
