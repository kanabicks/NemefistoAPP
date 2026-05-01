import { create } from "zustand";

export type ToastKind = "info" | "success" | "warning" | "error";

export type Toast = {
  id: number;
  kind: ToastKind;
  /** Заголовок (1-2 слова, моно). Можно опустить — будет только message. */
  title?: string;
  /** Основной текст. Поддерживается «\n» для второй строки. */
  message: string;
  /** Через сколько мс автоматически уйдёт. По умолчанию 5000. 0 — не уходит. */
  durationMs: number;
};

type ToastInput = Omit<Toast, "id" | "durationMs"> & { durationMs?: number };

type ToastStore = {
  toasts: Toast[];
  push: (t: ToastInput) => number;
  dismiss: (id: number) => void;
};

let nextId = 1;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],
  push: (input) => {
    const id = nextId++;
    const toast: Toast = {
      id,
      kind: input.kind,
      title: input.title,
      message: input.message,
      durationMs: input.durationMs ?? 5000,
    };
    set({ toasts: [...get().toasts, toast] });
    if (toast.durationMs > 0) {
      window.setTimeout(() => {
        get().dismiss(id);
      }, toast.durationMs);
    }
    return id;
  },
  dismiss: (id) => {
    set({ toasts: get().toasts.filter((t) => t.id !== id) });
  },
}));

/** Удобный helper для использования в компонентах. */
export const showToast = (input: ToastInput) =>
  useToastStore.getState().push(input);
