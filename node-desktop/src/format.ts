export type FieldKind = "value" | "unknown" | "unavailable";

export interface Field {
  kind: FieldKind;
  value?: string | null;
}

export function fieldText(field: Field | null | undefined): string {
  if (!field || field.kind !== "value") {
    return field?.kind === "unavailable" ? "unavailable" : "unknown";
  }
  if (field.value === undefined || field.value === null || field.value === "") {
    return "unknown";
  }
  return field.value;
}

export function pathsReady(binary: string, config: string, dataDir: string): boolean {
  return binary.trim() !== "" && config.trim() !== "" && dataDir.trim() !== "";
}

/** Missing success time is never "current". Explicit zero stays in `fieldText`. */
export function freshnessText(monitor: {
  rpc: string;
  stale: boolean;
  lastSuccessAgeMs: number | null;
}): string {
  if (monitor.lastSuccessAgeMs === null) {
    if (monitor.stale) return "stale; age unknown";
    if (monitor.rpc === "unreachable") return "unreachable";
    return "not observed";
  }
  if (monitor.stale) {
    return `stale last success ${Math.round(monitor.lastSuccessAgeMs / 1000)}s ago`;
  }
  if (monitor.rpc !== "reachable") return "unreachable";
  return "current";
}
