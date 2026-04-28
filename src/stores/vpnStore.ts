import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useSettingsStore } from "./settingsStore";

export type VpnStatus =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "error";

export type VpnMode = "proxy" | "tun";

type ConnectResult = {
  socks_port: number;
  http_port: number;
  server_name: string;
};

type VpnState = {
  status: VpnStatus;
  errorMessage: string | null;
  mode: VpnMode;
  selectedIndex: number | null;
  socksPort: number | null;
  httpPort: number | null;

  setMode: (mode: VpnMode) => void;
  selectServer: (index: number) => void;
  connect: () => Promise<void>;
  disconnect: () => Promise<void>;
  refresh: () => Promise<void>;
};

export const useVpnStore = create<VpnState>((set, get) => ({
  status: "stopped",
  errorMessage: null,
  mode: "proxy",
  selectedIndex: null,
  socksPort: null,
  httpPort: null,

  setMode: (mode) => set({ mode }),
  selectServer: (index) => set({ selectedIndex: index }),

  async refresh() {
    try {
      const running = await invoke<boolean>("is_xray_running");
      set((s) => ({
        status: running ? "running" : "stopped",
        errorMessage: null,
        socksPort: running ? s.socksPort : null,
        httpPort: running ? s.httpPort : null,
      }));
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },

  async connect() {
    const { selectedIndex, mode } = get();
    if (selectedIndex === null) return;

    const allowLan = useSettingsStore.getState().allowLan;
    set({ status: "starting", errorMessage: null });
    try {
      const result = await invoke<ConnectResult>("connect", {
        serverIndex: selectedIndex,
        mode,
        allowLan,
      });
      set({
        status: "running",
        socksPort: result.socks_port,
        httpPort: result.http_port,
        errorMessage: null,
      });
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },

  async disconnect() {
    set({ status: "stopping", errorMessage: null });
    try {
      await invoke("disconnect");
      set({
        status: "stopped",
        socksPort: null,
        httpPort: null,
        errorMessage: null,
      });
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },
}));
