import { useEffect, useState } from "react";
import { toast } from "sonner";

export function Receive({ address, pathLabel }: { address: string; pathLabel: string }) {
  const [qr, setQr] = useState<string | null>(null);

  useEffect(() => {
    let cancel = false;
    void import("qrcode").then((mod) =>
      mod.toDataURL(address, { margin: 1, width: 240, color: { dark: "#0C1412", light: "#F4F0E6" } }).then((url) => {
        if (!cancel) setQr(url);
      }),
    );
    return () => {
      cancel = true;
    };
  }, [address]);

  return (
    <div className="grid justify-items-center gap-4">
      <p className="text-sm text-muted">{pathLabel}</p>
      {qr ? <img src={qr} alt="Address QR" className="h-56 w-56 rounded-2xl bg-paper p-2" /> : <div className="h-56 w-56 rounded-2xl bg-ink" />}
      <p className="mono text-center text-sm break-all text-paper">{address}</p>
      <button
        type="button"
        className="min-h-11 rounded-full border border-line px-5 text-paper"
        onClick={() => {
          void navigator.clipboard.writeText(address);
          toast("Address copied");
        }}
      >
        Copy address
      </button>
    </div>
  );
}
