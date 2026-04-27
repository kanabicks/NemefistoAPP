import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export type ProxyEntry = {
  name: string;
  protocol: string;
  server: string;
  port: number;
  raw: Record<string, unknown>;
};

type SubscriptionStore = {
  servers: ProxyEntry[];
  loading: boolean;
  error: string | null;
  url: string;
  setUrl: (url: string) => void;
  fetchSubscription: () => Promise<void>;
  loadCached: () => Promise<void>;
};

export const useSubscriptionStore = create<SubscriptionStore>((set, get) => ({
  servers: [],
  loading: false,
  error: null,
  url: "",

  setUrl: (url) => set({ url }),

  async fetchSubscription() {
    const { url } = get();
    if (!url.trim()) return;
    set({ loading: true, error: null });
    try {
      const servers = await invoke<ProxyEntry[]>("fetch_subscription", { url });
      set({ servers, loading: false });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  async loadCached() {
    try {
      const servers = await invoke<ProxyEntry[]>("get_servers");
      if (servers.length > 0) set({ servers });
    } catch {
      // кеш пустой — не ошибка
    }
  },
}));
