import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useSettingsStore } from "./settingsStore";

export type ProxyEntry = {
  name: string;
  protocol: string;
  server: string;
  port: number;
  raw: Record<string, unknown>;
  /** Список движков, способных поднять этот сервер.
   *  Возможные значения: "xray", "mihomo".
   *  Если поле пустое (старый кеш до 8.A) — считаем совместимым с обоими. */
  engine_compat?: string[];
};

/** Метаданные подписки из HTTP-заголовков.
 *
 *  Стандартные (де-факто индустрии — 3x-ui / Marzban / x-ui / sing-box):
 *  - used/total: байты, total=0 → безлимит;
 *  - expireAt: unix-timestamp в секундах, null → бессрочно;
 *  - title: имя подписки (`profile-title`);
 *  - webPageUrl: URL личного кабинета (`profile-web-page-url`);
 *  - supportUrl: URL поддержки (`support-url`);
 *  - updateIntervalHours: интервал автообновления в часах
 *    (`profile-update-interval`);
 *  - announce / announceUrl: текст и опциональная ссылка для объявления
 *    от провайдера;
 *  - premiumUrl: URL премиум-страницы.
 *
 *  X-Nemefisto-* (наше расширение, server-driven UX, 8.C):
 *  - theme / background / buttonStyle / preset / mode / engine — задают
 *    дефолты; применяются только если пользователь не менял эти
 *    настройки вручную (override-логика).
 *
 *  Все enum-значения валидируются на бэкенде по whitelist; неизвестные
 *  становятся null. */
export type SubscriptionMeta = {
  used: number;
  total: number;
  expireAt: number | null;
  title: string | null;
  webPageUrl: string | null;
  supportUrl: string | null;
  updateIntervalHours: number | null;
  announce: string | null;
  announceUrl: string | null;
  premiumUrl: string | null;
  theme: string | null;
  background: string | null;
  buttonStyle: string | null;
  preset: string | null;
  mode: string | null;
  engine: string | null;
  // Anti-DPI (этап 10)
  fragmentationEnable: boolean | null;
  fragmentationPackets: string | null;
  fragmentationLength: string | null;
  fragmentationInterval: string | null;
  noisesEnable: boolean | null;
  noisesType: string | null;
  noisesPacket: string | null;
  noisesDelay: string | null;
  serverResolveEnable: boolean | null;
  serverResolveDoH: string | null;
  serverResolveBootstrap: string | null;
};

/** Сырой ответ команды fetch_subscription — Rust возвращает snake_case. */
type SubscriptionMetaRaw = {
  used: number;
  total: number;
  expire_at: number | null;
  title: string | null;
  web_page_url: string | null;
  support_url: string | null;
  update_interval_hours: number | null;
  announce: string | null;
  announce_url: string | null;
  premium_url: string | null;
  theme: string | null;
  background: string | null;
  button_style: string | null;
  preset: string | null;
  mode: string | null;
  engine: string | null;
  fragmentation_enable: boolean | null;
  fragmentation_packets: string | null;
  fragmentation_length: string | null;
  fragmentation_interval: string | null;
  noises_enable: boolean | null;
  noises_type: string | null;
  noises_packet: string | null;
  noises_delay: string | null;
  server_resolve_enable: boolean | null;
  server_resolve_doh: string | null;
  server_resolve_bootstrap: string | null;
};
type FetchSubscriptionRaw = {
  servers: ProxyEntry[];
  meta: SubscriptionMetaRaw | null;
};

type SubscriptionStore = {
  servers: ProxyEntry[];
  /** Метаданные подписки или null если сервер их не прислал. */
  meta: SubscriptionMeta | null;
  /** Пинги по индексам серверов: ms или null если offline / timeout. */
  pings: (number | null)[];
  pingsLoading: boolean;
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
  pingAll: () => Promise<void>;
};

/** Конверсия snake_case ответа Rust → camelCase TS. */
const normalizeMeta = (
  raw: SubscriptionMetaRaw | null
): SubscriptionMeta | null =>
  raw
    ? {
        used: raw.used,
        total: raw.total,
        expireAt: raw.expire_at,
        title: raw.title,
        webPageUrl: raw.web_page_url,
        supportUrl: raw.support_url,
        updateIntervalHours: raw.update_interval_hours,
        announce: raw.announce,
        announceUrl: raw.announce_url,
        premiumUrl: raw.premium_url,
        theme: raw.theme,
        background: raw.background,
        buttonStyle: raw.button_style,
        preset: raw.preset,
        mode: raw.mode,
        engine: raw.engine,
        fragmentationEnable: raw.fragmentation_enable,
        fragmentationPackets: raw.fragmentation_packets,
        fragmentationLength: raw.fragmentation_length,
        fragmentationInterval: raw.fragmentation_interval,
        noisesEnable: raw.noises_enable,
        noisesType: raw.noises_type,
        noisesPacket: raw.noises_packet,
        noisesDelay: raw.noises_delay,
        serverResolveEnable: raw.server_resolve_enable,
        serverResolveDoH: raw.server_resolve_doh,
        serverResolveBootstrap: raw.server_resolve_bootstrap,
      }
    : null;

const URL_KEY = "nemefisto.subscription.url";
// Версионируем ключ override-HWID: при апгрейде клиента старое значение
// (когда мы вручную подсовывали Happ-овский HWID для отладки) автоматически
// перестаёт читаться. Override — теперь advanced-only, по умолчанию используется
// системный MachineGuid через get_hwid.
const HWID_KEY = "nemefisto.subscription.hwid.v2";
const HWID_KEY_LEGACY = "nemefisto.subscription.hwid";
const MIGRATION_KEY = "nemefisto.migrated.v2";

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

// Чистим устаревший ключ override-HWID. Версионирование выше уже отрезает
// его от чтения, но удаляем для гигиены localStorage.
const runMigrations = () => {
  try {
    if (!localStorage.getItem(MIGRATION_KEY)) {
      localStorage.removeItem(HWID_KEY_LEGACY);
      localStorage.setItem(MIGRATION_KEY, "1");
    }
  } catch {
    // приватный режим — пропускаем
  }
};
runMigrations();

export const useSubscriptionStore = create<SubscriptionStore>((set, get) => ({
  servers: [],
  meta: null,
  pings: [],
  pingsLoading: false,
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
    const { userAgent, sendHwid } = useSettingsStore.getState();
    set({ loading: true, error: null });
    try {
      const result = await invoke<FetchSubscriptionRaw>("fetch_subscription", {
        url,
        hwidOverride: hwid.trim() || null,
        userAgent: userAgent.trim() || null,
        sendHwid,
      });
      set({
        servers: result.servers,
        meta: normalizeMeta(result.meta),
        pings: [],
        loading: false,
      });
      // Авто-пинг сразу после получения списка
      void get().pingAll();
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  async loadCached() {
    try {
      const servers = await invoke<ProxyEntry[]>("get_servers");
      if (servers.length > 0) {
        set({ servers });
        // Метаданные кешируются параллельно — могут отсутствовать если
        // сервер их не присылал.
        try {
          const rawMeta = await invoke<SubscriptionMetaRaw | null>(
            "get_subscription_meta"
          );
          set({ meta: normalizeMeta(rawMeta) });
        } catch {
          // не критично
        }
        void get().pingAll();
      }
    } catch {
      // кеш пустой — не ошибка
    }
  },

  async pingAll() {
    if (get().servers.length === 0) return;
    set({ pingsLoading: true });
    try {
      const result = await invoke<(number | null)[]>("ping_servers");
      set({ pings: result, pingsLoading: false });
    } catch {
      set({ pingsLoading: false });
    }
  },
}));
