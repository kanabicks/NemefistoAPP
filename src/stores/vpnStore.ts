import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useSettingsStore } from "./settingsStore";
import { useSubscriptionStore } from "./subscriptionStore";

/** Anti-DPI опции в формате camelCase, который Rust десериализует через
 *  serde(rename_all = "camelCase") в struct AntiDpiOptions. */
type AntiDpiPayload = {
  fragmentation: boolean;
  fragmentationPackets: string;
  fragmentationLength: string;
  fragmentationInterval: string;
  noises: boolean;
  noisesType: string;
  noisesPacket: string;
  noisesDelay: string;
  serverResolve: boolean;
  serverResolveDoH: string;
  serverResolveBootstrap: string;
};

/** Effective anti-DPI с учётом override-логики 8.C: если пользователь
 *  не трогал, используются значения из заголовков подписки. Возвращает
 *  null если все три механизма выключены — connect передаст None. */
function buildEffectiveAntiDpi(): AntiDpiPayload | null {
  const s = useSettingsStore.getState();
  const meta = useSubscriptionStore.getState().meta;
  const touched = s.antiDpiTouched;

  // Boolean: from header if untouched и заголовок прислал значение,
  // иначе from settings.
  const pickBool = (
    metaVal: boolean | null | undefined,
    settingVal: boolean
  ): boolean =>
    !touched && metaVal != null ? metaVal : settingVal;
  const pickStr = (
    metaVal: string | null | undefined,
    settingVal: string
  ): string => (!touched && metaVal ? metaVal : settingVal);

  const result: AntiDpiPayload = {
    fragmentation: pickBool(meta?.fragmentationEnable, s.antiDpiFragmentation),
    fragmentationPackets: pickStr(
      meta?.fragmentationPackets,
      s.antiDpiFragmentationPackets
    ),
    fragmentationLength: pickStr(
      meta?.fragmentationLength,
      s.antiDpiFragmentationLength
    ),
    fragmentationInterval: pickStr(
      meta?.fragmentationInterval,
      s.antiDpiFragmentationInterval
    ),
    noises: pickBool(meta?.noisesEnable, s.antiDpiNoises),
    noisesType: pickStr(meta?.noisesType, s.antiDpiNoisesType),
    noisesPacket: pickStr(meta?.noisesPacket, s.antiDpiNoisesPacket),
    noisesDelay: pickStr(meta?.noisesDelay, s.antiDpiNoisesDelay),
    serverResolve: pickBool(
      meta?.serverResolveEnable,
      s.antiDpiServerResolve
    ),
    serverResolveDoH: pickStr(meta?.serverResolveDoH, s.antiDpiResolveDoH),
    serverResolveBootstrap: pickStr(
      meta?.serverResolveBootstrap,
      s.antiDpiResolveBootstrap
    ),
  };

  // Если ни один механизм не включён — не платим за лишний JSON-сериализатор
  // в Rust, передаём null (anti_dpi: None).
  if (!result.fragmentation && !result.noises && !result.serverResolve) {
    return null;
  }
  return result;
}

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
  /** SOCKS5 username/password для LAN-режима (этап 9.G).
   *  Заполнено только когда LAN активен; UI показывает их с copy-кнопкой. */
  socks_username?: string | null;
  socks_password?: string | null;
};

type VpnState = {
  status: VpnStatus;
  errorMessage: string | null;
  mode: VpnMode;
  selectedIndex: number | null;
  socksPort: number | null;
  httpPort: number | null;
  /** SOCKS5 креды показываемые в LAN-режиме (этап 9.G).
   *  null когда LAN выключен или connect ещё не выполнялся. */
  socksUsername: string | null;
  socksPassword: string | null;

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
  socksUsername: null,
  socksPassword: null,

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
    const tunMasking = useSettingsStore.getState().tunMasking;
    const antiDpi = buildEffectiveAntiDpi();
    set({ status: "starting", errorMessage: null });
    try {
      const result = await invoke<ConnectResult>("connect", {
        serverIndex: selectedIndex,
        mode,
        allowLan,
        antiDpi,
        tunMasking,
      });
      set({
        status: "running",
        socksPort: result.socks_port,
        httpPort: result.http_port,
        socksUsername: result.socks_username ?? null,
        socksPassword: result.socks_password ?? null,
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
        socksUsername: null,
        socksPassword: null,
        errorMessage: null,
      });
    } catch (e) {
      set({ status: "error", errorMessage: String(e) });
    }
  },
}));
