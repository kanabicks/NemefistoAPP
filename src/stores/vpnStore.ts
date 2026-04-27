import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export type VpnStatus =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "error";

type VpnState = {
  status: VpnStatus;
  errorMessage: string | null;
  refresh: () => Promise<void>;
  start: () => Promise<void>;
  stop: () => Promise<void>;
};

export const useVpnStore = create<VpnState>((set, get) => ({
  status: "stopped",
  errorMessage: null,

  async refresh() {
    try {
      const running = await invoke<boolean>("is_xray_running");
      set({
        status: running ? "running" : "stopped",
        errorMessage: null,
      });
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },

  async start() {
    set({ status: "starting", errorMessage: null });
    try {
      await invoke("start_xray");
      await get().refresh();
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },

  async stop() {
    set({ status: "stopping", errorMessage: null });
    try {
      await invoke("stop_xray");
      await get().refresh();
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },
}));
