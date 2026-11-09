const ATOMS = 1_000_000_000n;

export function parseKvnc(input: string): bigint | null {
  const text = input.trim();
  if (!/^\d+(\.\d{0,9})?$/.test(text)) return null;
  const [whole, frac = ""] = text.split(".");
  if (whole!.length > 1 && whole!.startsWith("0")) return null;
  const atoms = BigInt(whole) * ATOMS + BigInt(frac.padEnd(9, "0") || "0");
  if (atoms < 0n || atoms > 0xffffffffffffffffn) return null;
  return atoms;
}

export function formatKvnc(atoms: bigint): string {
  const negative = atoms < 0n;
  const value = negative ? -atoms : atoms;
  const whole = value / ATOMS;
  const frac = (value % ATOMS).toString().padStart(9, "0").replace(/0+$/, "");
  const text = frac ? `${whole}.${frac}` : whole.toString();
  return negative ? `-${text}` : text;
}

export function formatAtoms(atoms: bigint): string {
  return atoms.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

export function parseAtoms(input: string): bigint | null {
  if (!/^\d+$/.test(input.trim())) return null;
  try {
    const value = BigInt(input.trim());
    if (value > 0xffffffffffffffffn) return null;
    return value;
  } catch {
    return null;
  }
}
