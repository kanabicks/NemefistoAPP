import { create } from "zustand";

type ExampleState = {
  counter: number;
  increment: () => void;
  reset: () => void;
};

export const useExampleStore = create<ExampleState>((set) => ({
  counter: 0,
  increment: () => set((state) => ({ counter: state.counter + 1 })),
  reset: () => set({ counter: 0 }),
}));
