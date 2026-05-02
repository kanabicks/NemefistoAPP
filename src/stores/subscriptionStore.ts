import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { effectiveUserAgent, useSettingsStore } from "./settingsStore";
import { useVpnStore } from "./vpnStore";

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
  // 11.E: routing-директивы из спец-строк подписки. UI применяет
  // через invoke routing_add_url / routing_add_static + опционально
  // routing_set_active.
  routingAutorouting: [string, boolean] | null;
  routingStatic: [string, boolean] | null;
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
  routing_autorouting: [string, boolean] | null;
  routing_static: [string, boolean] | null;
};
type FetchSubscriptionRaw = {
  servers: ProxyEntry[];
  meta: SubscriptionMetaRaw | null;
};

type SubscriptionStore = {
  servers: ProxyEntry[];
  /** Метаданные подписки или null если сервер их не прислал. */
  meta: SubscriptionMeta | null;
  /** Unix-ms времени последнего успешного fetchSubscription. null —
   *  ни разу не обновлялась за всю жизнь приложения (например, серверы
   *  пришли только из кеша при старте). 12.B */
  lastFetchedAt: number | null;
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
  /** Прочитать URL/HWID из Windows Credential Manager. При первом запуске
   *  мигрирует значения из localStorage → keyring и удаляет их из
   *  localStorage. См. этап 6.A. */
  loadSecureCreds: () => Promise<void>;
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
        routingAutorouting: raw.routing_autorouting,
        routingStatic: raw.routing_static,
      }
    : null;

/** Ключи в Windows Credential Manager (этап 6.A). Чувствительные данные
 *  переехали из localStorage в защищённое хранилище ОС. localStorage
 *  ключи остаются как fallback на время миграции. */
const URL_KEYRING = "subscription_url";
const HWID_KEYRING = "hwid_override";

const URL_KEY = "nemefisto.subscription.url";
const LAST_FETCH_KEY = "nemefisto.subscription.lastFetchedAt";
const KEYRING_MIGRATION_KEY = "nemefisto.migrated.keyring.v1";
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

/** Записать значение в Windows Credential Manager. Возвращает true при
 *  успехе. Ошибки не критичны — на платформах без keyring (или приватных
 *  пользователях) тихо проваливаемся. */
const keyringSet = async (key: string, value: string): Promise<boolean> => {
  try {
    await invoke("secure_storage_set", { key, value });
    return true;
  } catch {
    return false;
  }
};

const keyringGet = async (key: string): Promise<string> => {
  try {
    return await invoke<string>("secure_storage_get", { key });
  } catch {
    return "";
  }
};

const keyringDelete = async (key: string): Promise<void> => {
  try {
    await invoke("secure_storage_delete", { key });
  } catch {
    // ignore
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

/** Прочитать unix-ms из localStorage. Возвращает null если ключа нет
 *  или значение некорректное. */
const loadTimestamp = (key: string): number | null => {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const n = Number(raw);
    return Number.isFinite(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
};

export const useSubscriptionStore = create<SubscriptionStore>((set, get) => ({
  servers: [],
  meta: null,
  lastFetchedAt: loadTimestamp(LAST_FETCH_KEY),
  pings: [],
  pingsLoading: false,
  loading: false,
  error: null,
  url: loadFromStorage(URL_KEY),
  deviceHwid: "",
  hwid: loadFromStorage(HWID_KEY),

  setUrl: (url) => {
    set({ url });
    // Чувствительные значения пишем в Windows Credential Manager.
    // localStorage больше НЕ используется как источник правды — оставляем
    // пустым, чтобы старые версии приложения не подсунули устаревший URL.
    saveToStorage(URL_KEY, "");
    void keyringSet(URL_KEYRING, url);
  },
  setHwid: (hwid) => {
    set({ hwid });
    saveToStorage(HWID_KEY, "");
    if (hwid.trim()) {
      void keyringSet(HWID_KEYRING, hwid);
    } else {
      void keyringDelete(HWID_KEYRING);
    }
  },

  async loadDeviceHwid() {
    try {
      const id = await invoke<string>("get_hwid");
      set({ deviceHwid: id });
    } catch {
      // не критично — UI покажет пустую строку
    }
  },

  async loadSecureCreds() {
    // Этап 6.A: читаем URL/HWID из Windows Credential Manager. Если в
    // keyring пусто, но в localStorage есть — мигрируем (один раз) и
    // зачищаем localStorage. Маркер миграции защищает от повторного
    // запуска (если вдруг пользователь вернёт старую версию и оставит
    // там URL — на следующем апгрейде не будем перезатирать keyring).
    let migrated = false;
    try {
      migrated = !!localStorage.getItem(KEYRING_MIGRATION_KEY);
    } catch {
      // приватный режим — мигрируем каждый раз, не критично
    }

    let urlFromKeyring = await keyringGet(URL_KEYRING);
    let hwidFromKeyring = await keyringGet(HWID_KEYRING);

    if (!migrated) {
      const legacyUrl = loadFromStorage(URL_KEY);
      const legacyHwid = loadFromStorage(HWID_KEY);
      if (!urlFromKeyring && legacyUrl) {
        await keyringSet(URL_KEYRING, legacyUrl);
        urlFromKeyring = legacyUrl;
      }
      if (!hwidFromKeyring && legacyHwid) {
        await keyringSet(HWID_KEYRING, legacyHwid);
        hwidFromKeyring = legacyHwid;
      }
      saveToStorage(URL_KEY, "");
      saveToStorage(HWID_KEY, "");
      try {
        localStorage.setItem(KEYRING_MIGRATION_KEY, "1");
      } catch {
        // ignore
      }
    }

    if (urlFromKeyring) {
      set({ url: urlFromKeyring });
    }
    if (hwidFromKeyring) {
      set({ hwid: hwidFromKeyring });
    }
  },

  async fetchSubscription() {
    const { url, hwid } = get();
    if (!url.trim()) return;
    const settings = useSettingsStore.getState();
    // 8.B: эффективный UA зависит от движка. Если пользователь не правил
    // поле — Xray идёт с Happ-UA (получаем Marzban xray-json с готовым
    // routing'ом), Mihomo — с clash-verge UA (получаем clash YAML с
    // соответствующими правилами). Когда пользователь правил вручную —
    // используется как есть, без перезаписи.
    const ua = effectiveUserAgent(
      settings.engine,
      settings.userAgent,
      settings.userAgentTouched
    );
    set({ loading: true, error: null });
    try {
      const result = await invoke<FetchSubscriptionRaw>("fetch_subscription", {
        url,
        hwidOverride: hwid.trim() || null,
        userAgent: ua.trim() || null,
        sendHwid: settings.sendHwid,
      });
      const now = Date.now();
      saveToStorage(LAST_FETCH_KEY, String(now));
      const normalized = normalizeMeta(result.meta);
      set({
        servers: result.servers,
        meta: normalized,
        pings: [],
        lastFetchedAt: now,
        loading: false,
      });
      // 0.1.2: при смене движка/подписки список серверов мог сильно
      // сократиться (например xray-flat 5 серверов → mihomo-passthrough
      // 1 синтетическая запись). Если selectedIndex теперь вне
      // диапазона — сбрасываем, иначе следующий connect упадёт на
      // «сервер #N не найден в списке». Также чистим если новый список
      // пустой (нет смысла держать индекс).
      //
      // 0.1.2: если в списке ровно один сервер — выбираем его
      // автоматом. Это критично для mihomo-passthrough (единственная
      // запись «Профиль Mihomo» — без неё MihomoGroupsInline не
      // отрисуется и пользователь видит только пустой ServerSelector
      // вместо групп). Для обычных подписок с 1 нодой тоже полезно —
      // выбирать там нечего, лишний клик ради тика галочки.
      {
        const vpn = useVpnStore.getState();
        if (
          vpn.selectedIndex !== null &&
          (result.servers.length === 0 ||
            vpn.selectedIndex >= result.servers.length)
        ) {
          useVpnStore.setState({ selectedIndex: null });
        }
        if (
          useVpnStore.getState().selectedIndex === null &&
          result.servers.length === 1
        ) {
          useVpnStore.setState({ selectedIndex: 0 });
        }
      }
      // Авто-пинг сразу после получения списка
      void get().pingAll();

      // 11.E: если в подписке нашлись routing-директивы (`://routing/...`,
      // `://autorouting/...` спец-строки) — применяем через bash-команды.
      // Не блокируем основной flow — выполняем в фоне.
      if (normalized?.routingAutorouting) {
        const [autoUrl, activate] = normalized.routingAutorouting;
        void invoke<string>("routing_add_url", {
          url: autoUrl,
          intervalHours: 24,
        })
          .then((id) =>
            activate
              ? invoke("routing_set_active", { id })
              : Promise.resolve()
          )
          .catch((e) =>
            console.warn("[subscription] routing_autorouting failed:", e)
          );
      }
      if (normalized?.routingStatic) {
        const [payload, activate] = normalized.routingStatic;
        const isUrl = /^https?:\/\//i.test(payload);
        const promise = isUrl
          ? invoke<string>("routing_add_url", {
              url: payload,
              intervalHours: 8760, // эффективное «не обновлять»
            })
          : invoke<string>("routing_add_static", { payload });
        void promise
          .then((id) =>
            activate
              ? invoke("routing_set_active", { id })
              : Promise.resolve()
          )
          .catch((e) =>
            console.warn("[subscription] routing_static failed:", e)
          );
      }
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  async loadCached() {
    try {
      const servers = await invoke<ProxyEntry[]>("get_servers");
      if (servers.length > 0) {
        set({ servers });
        // 0.1.2: тот же sanity-check что в fetchSubscription —
        // если кешированный selectedIndex вне нового диапазона, чистим.
        // И auto-select 0 когда сервер единственный (mihomo-passthrough).
        const vpn = useVpnStore.getState();
        if (vpn.selectedIndex !== null && vpn.selectedIndex >= servers.length) {
          useVpnStore.setState({ selectedIndex: null });
        }
        if (
          useVpnStore.getState().selectedIndex === null &&
          servers.length === 1
        ) {
          useVpnStore.setState({ selectedIndex: 0 });
        }
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
