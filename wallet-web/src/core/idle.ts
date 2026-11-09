export function remainingMs(lockMinutes: number, at: number, now: number): number {
  if (!Number.isFinite(lockMinutes) || lockMinutes <= 0) return 0;
  return Math.max(0, lockMinutes * 60 * 1000 - (now - at));
}

export function formatRemaining(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}
