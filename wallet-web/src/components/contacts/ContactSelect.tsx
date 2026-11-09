import { useEffect, useState } from "react";
import { loadContacts, type Contact } from "@/stores/contacts";

export function ContactSelect({ onPick }: { onPick: (address: string) => void }) {
  const [contacts, setContacts] = useState<Contact[]>([]);
  useEffect(() => setContacts(loadContacts()), []);
  if (contacts.length === 0) return null;
  return (
    <label className="grid gap-1 text-sm text-muted">
      Pick contact
      <select
        className="min-h-11 rounded-2xl border border-line bg-ink px-4 text-paper"
        defaultValue=""
        onChange={(event) => {
          const found = contacts.find((item) => item.id === event.target.value);
          if (found) onPick(found.address);
        }}
      >
        <option value="">Choose a saved address</option>
        {contacts.map((contact) => (
          <option key={contact.id} value={contact.id}>
            {contact.label}
          </option>
        ))}
      </select>
    </label>
  );
}
