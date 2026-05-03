import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import i18n from "../i18n";
import { useSettingsStore } from "./settingsStore";
import { useSubscriptionStore } from "./subscriptionStore";
import { showToast } from "./toastStore";

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
  selectServer: (index) => {
    const prev = get().selectedIndex;
    set({ selectedIndex: index });
    // 0.1.1 / Bug 6: авто-reconnect при смене сервера. Раньше после
    // выбора другого сервера пользователь должен был вручную
    // disconnect → connect — счёт-фактура «два клика на смену сервера»
    // была неудобной.
    //
    // Теперь: если VPN активен и индекс реально сменился, мы атомарно
    // переподключаемся к новому серверу. Если был только что error /
    // stopping / starting — не трогаем (пусть пользователь дождётся
    // финального состояния).
    if (prev !== null && prev !== index) {
      const status = get().status;
      if (status === "running") {
        // disconnect → пауза 200мс на снятие WFP/маршрутов → connect.
        // Без паузы новый коннект может не дождаться очистки и
        // увидеть «old proxy still active».
        void (async () => {
          showToast({
            kind: "info",
            title: i18n.t("vpnStore.switching.title"),
            message: i18n.t("vpnStore.switching.message"),
            durationMs: 3000,
          });
          await get().disconnect();
          await new Promise((r) => setTimeout(r, 200));
          await get().connect();
        })();
      }
    }
  },

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

    // 9.C: проверяем routing-таблицу на чужие default/half-default
    // маршруты до запуска connect. Если такие есть — это другой
    // активный VPN, и наш TUN/прокси конфликтует с ним. Не запускаем.
    try {
      const conflicts = await invoke<string[]>("check_routing_conflicts");
      if (Array.isArray(conflicts) && conflicts.length > 0) {
        const list = conflicts.join(", ");
        showToast({
          kind: "warning",
          title: i18n.t("vpnStore.vpnConflict.title"),
          message: i18n.t("vpnStore.vpnConflict.message", { list }),
          durationMs: 8000,
        });
        return;
      }
    } catch {
      // Не критично: detect best-effort, не должен блокировать connect
      // если внутри Win32 что-то отказало.
    }

    const allowLan = useSettingsStore.getState().allowLan;
    const tunMasking = useSettingsStore.getState().tunMasking;
    const killSwitch = useSettingsStore.getState().killSwitch;
    const killSwitchStrict =
      useSettingsStore.getState().killSwitchStrict;
    const autoApplyMinimalRuRules =
      useSettingsStore.getState().autoApplyMinimalRuRules;
    const dnsLeakProtection =
      useSettingsStore.getState().dnsLeakProtection;
    const antiDpi = buildEffectiveAntiDpi();
    // 8.D: per-process правила. Подаём в Rust в camelCase
    // (`exe`/`action`/`comment`); serde на стороне Rust десериализует
    // в `AppRule`. Если правил нет — пустой массив, ветка mihomo
    // воспримет его как «no PROCESS-NAME правил» и не включит
    // дорогой `find-process-mode: always`.
    const appRules = useSettingsStore.getState().appRules;
    // 8.B/8.C: эффективный engine. Если пользователь явно не менял
    // (engineTouched=false) и подписка прислала X-Nemefisto-Engine —
    // берём из заголовка; иначе — пользовательский выбор.
    const settings = useSettingsStore.getState();
    const meta = useSubscriptionStore.getState().meta;
    // sing-box миграция (0.1.2): подписка может всё ещё присылать
    // `X-Nemefisto-Engine: xray` — мы автоматически маппим в "sing-box"
    // (sing-box покрывает всё что покрывал xray, без потери семантики).
    const headerEngineRaw = meta?.engine;
    const headerEngine: "sing-box" | "mihomo" | null =
      headerEngineRaw === "mihomo"
        ? "mihomo"
        : headerEngineRaw === "sing-box" || headerEngineRaw === "xray"
        ? "sing-box"
        : null;
    const engine: "sing-box" | "mihomo" =
      !settings.engineTouched && headerEngine
        ? headerEngine
        : settings.engine;
    set({ status: "starting", errorMessage: null });
    try {
      const result = await invoke<ConnectResult>("connect", {
        serverIndex: selectedIndex,
        mode,
        engine,
        allowLan,
        antiDpi,
        tunMasking,
        killSwitch,
        dnsLeakProtection,
        killSwitchStrict,
        autoApplyMinimalRuRules,
        appRules,
      });
      set({
        status: "running",
        socksPort: result.socks_port,
        httpPort: result.http_port,
        socksUsername: result.socks_username ?? null,
        socksPassword: result.socks_password ?? null,
        errorMessage: null,
      });
      // 8.F: для mihomo-движка применяем сохранённые пользователем
      // preferredMihomoNodes (см. ProxiesPanel — клик по ноде до connect
      // запоминает её как предпочитаемую). external-controller только
      // что поднялся — Rust сохранил endpoint в connect, теперь дёргаем
      // /proxies/:group для каждой записи. Ошибки глотаем — обычно это
      // означает что имя группы не совпадает (пользователь сменил
      // подписку). UI panel дальше работает в live-режиме как обычно.
      if (engine === "mihomo") {
        const preferred = useSettingsStore.getState().preferredMihomoNodes;
        for (const [group, name] of Object.entries(preferred)) {
          try {
            await invoke("mihomo_select_proxy", { group, name });
          } catch {
            // имя группы/ноды устарело — игнорируем
          }
        }
      }
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
