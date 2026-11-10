import { create } from "zustand";

type IdleState = {
  at: number;
  bump: () => void;
};

export const useIdle = create<IdleState>((set) => ({
  at: Date.now(),
  bump: () => set({ at: Date.now() }),
}));
