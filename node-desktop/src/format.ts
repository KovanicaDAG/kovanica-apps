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
