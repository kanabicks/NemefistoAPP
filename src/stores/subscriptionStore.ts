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
  /** HWID устройства (читается из Windows MachineGuid). Auto, read-only. */
  deviceHwid: string;
  /** Опциональный override HWID для разработки / переноса с другого клиента. */
  hwid: string;
  setUrl: (url: string) => void;
  setHwid: (hwid: string) => void;
  loadDeviceHwid: () => Promise<void>;
  fetchSubscription: () => Promise<void>;
  loadCached: () => Promise<void>;
};

const URL_KEY = "nemefisto.subscription.url";
const HWID_KEY = "nemefisto.subscription.hwid";

const loadFromStorage = (key: string): string => {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
};

const saveToStorage = (key: string, value: string) => {
  try {
    localStorage.setItem(key, value);
  } catch {
    // приватный режим/квота — не критично
  }
};

export const useSubscriptionStore = create<SubscriptionStore>((set, get) => ({
  servers: [],
  loading: false,
  error: null,
  url: loadFromStorage(URL_KEY),
  deviceHwid: "",
  hwid: loadFromStorage(HWID_KEY),

  setUrl: (url) => {
    saveToStorage(URL_KEY, url);
    set({ url });
  },
  setHwid: (hwid) => {
    saveToStorage(HWID_KEY, hwid);
    set({ hwid });
  },

  async loadDeviceHwid() {
    try {
      const id = await invoke<string>("get_hwid");
      set({ deviceHwid: id });
    } catch {
      // не критично — UI покажет пустую строку
    }
  },

  async fetchSubscription() {
    const { url, hwid } = get();
    if (!url.trim()) return;
    set({ loading: true, error: null });
    try {
      const servers = await invoke<ProxyEntry[]>("fetch_subscription", {
        url,
        hwidOverride: hwid.trim() || null,
      });
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
