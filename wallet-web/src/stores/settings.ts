import { create } from "zustand";

const KEY = "kvnc.settings.v1";
const CHOICES = [1, 5, 15, 60] as const;
export type LockMinutes = (typeof CHOICES)[number];

function readMinutes(): LockMinutes {
  if (typeof localStorage === "undefined") return 5;
  const raw = Number(localStorage.getItem(KEY));
  return CHOICES.includes(raw as LockMinutes) ? (raw as LockMinutes) : 5;
}

type SettingsState = {
  lockMinutes: LockMinutes;
  setLockMinutes: (minutes: LockMinutes) => void;
};

export const useSettings = create<SettingsState>((set) => ({
  lockMinutes: readMinutes(),
  setLockMinutes: (lockMinutes) => {
    localStorage.setItem(KEY, String(lockMinutes));
    set({ lockMinutes });
  },
}));

export const LOCK_CHOICES = CHOICES;
