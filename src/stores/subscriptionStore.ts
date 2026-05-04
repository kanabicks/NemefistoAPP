import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { effectiveUserAgent, useSettingsStore } from "./settingsStore";
import { findSelectedIndexByName, useVpnStore } from "./vpnStore";
import i18n from "../i18n";
import { showToast } from "./toastStore";

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
  /** 0.3.0 multi-subscription: id подписки-источника. Заполняется
   *  frontend'ом при сохранении в state.servers (Rust возвращает entries
   *  без этого поля). Используется в drawer'е для group-by-source и в
   *  vpn_connect для выбора правильного engine через
   *  `getEffectiveEngine(subscriptionId)`. */
  subscriptionId?: string;
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

/** 0.3.0 (multi-subscription): описание одной подписки. До 0.3.0 был
 *  один URL/HWID/meta в singleton-полях; теперь — массив `subscriptions`,
 *  каждая со своими данными. Legacy-поля (url/hwid/meta/...) остаются для
 *  backward compat: они sync'ятся с **primary** подпиской (subscriptions[0]
 *  по умолчанию). UI постепенно переезжает на чтение `subscriptions`. */
export type Subscription = {
  /** uuid v4. Используется как суффикс keyring-ключей и subscriptionId
   *  у `ProxyEntry` (для group-by-source в drawer). */
  id: string;
  url: string;
  hwid: string;
  /** Метаданные одной подписки (трафик, срок, заголовки X-Nemefisto-*).
   *  null если ещё ни разу не fetch'или или провайдер не прислал. */
  meta: SubscriptionMeta | null;
  /** Unix-ms времени последнего успешного fetch для этой подписки. */
  lastFetchedAt: number | null;
  loading: boolean;
  error: string | null;
  /** Per-subscription engine override через ⋯ меню карточки.
   *  null = авто (header X-Nemefisto-Engine → settings.engine fallback). */
  engineOverride: "sing-box" | "mihomo" | null;
  /** 0.3.0 multi-server-list: серверы этой подписки. Tagged
   *  с subscriptionId для group-by-source. */
  servers: ProxyEntry[];
  /** Пинги по индексам servers (parallel array). */
  pings: (number | null)[];
};

type SubscriptionStore = {
  // ─── Multi-subscription API (0.3.0+) ─────────────────────────────────
  /** Все подписки. Первая = primary (для legacy совместимости). */
  subscriptions: Subscription[];
  /** id текущей primary подписки. null когда subscriptions пустой. */
  primaryId: string | null;
  /** Добавить новую подписку с заданным URL. Создаёт uuid, сохраняет URL
   *  в keyring под `subscription_url:${id}`, делает fetch, возвращает id. */
  addSubscription: (url: string) => Promise<string>;
  /** Удалить подписку (по id). Если удаляем primary — следующая в списке
   *  становится primary. Чистит keyring entries и связанные servers. */
  removeSubscription: (id: string) => Promise<void>;
  /** Назначить primary (обычно UI-переключатель в Welcome / Settings). */
  setPrimaryId: (id: string) => void;
  /** Set engineOverride для подписки (через ⋯ меню → radio выбор). */
  setEngineOverride: (
    id: string,
    engine: "sing-box" | "mihomo" | null
  ) => void;
  /** Получить effective engine для подписки: override → header → fallback. */
  getEffectiveEngine: (id: string) => "sing-box" | "mihomo";
  /** Fetch конкретной подписки. Если эта подписка primary, синхронизирует
   *  legacy state.servers/meta для backward compat. Если не primary —
   *  обновляет только sub.servers/meta. Internally делает swap+restore
   *  primary, чтобы Rust state в момент connect содержал нужные servers. */
  fetchSubscriptionById: (id: string) => Promise<void>;
  /** Пинги серверов конкретной подписки. */
  pingAllOf: (id: string) => Promise<void>;

  // ─── Legacy API (sync'ится с primary, остаётся для обратной совместимости)
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
  /** Полная очистка подписки: URL/HWID из keyring, кеш серверов в Rust,
   *  meta + lastFetched + pings в памяти, persisted selectedIndex name.
   *  После вызова экран возвращается к Welcome. Эквивалент
   *  removeSubscription(primaryId), оставлен для существующих callsites. */
  deleteSubscription: () => Promise<void>;
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

/** localStorage key для списка ID-всех-подписок (multi-subscription
 *  state). Каждый id — uuid v4. Соответствующие URL/HWID хранятся в
 *  Windows Credential Manager под ключами `subscription_url:${id}` и
 *  `hwid_override:${id}`. На init читаем этот список → подгружаем по
 *  каждому id креды из keyring. */
const SUBS_INDEX_KEY = "nemefisto.subscriptions.index.v1";

const loadSubsIndex = (): string[] => {
  try {
    const raw = localStorage.getItem(SUBS_INDEX_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
};

const saveSubsIndex = (ids: string[]) => {
  try {
    localStorage.setItem(SUBS_INDEX_KEY, JSON.stringify(ids));
  } catch {
    // приватный режим — не критично
  }
};

const genId = (): string => {
  // Безопасный uuid v4 без external crypto.randomUUID для старых runtime'ов.
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return "sub-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 10);
};

const newSubscription = (url: string, hwid: string): Subscription => ({
  id: genId(),
  url,
  hwid,
  meta: null,
  lastFetchedAt: null,
  loading: false,
  error: null,
  engineOverride: null,
  servers: [],
  pings: [],
});

export const useSubscriptionStore = create<SubscriptionStore>((set, get) => ({
  // Multi-subscription (0.3.0+) — populated from `loadSecureCreds`.
  subscriptions: [],
  primaryId: null,

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

  // ─── Multi-subscription methods (0.3.0+) ─────────────────────────────

  async addSubscription(url) {
    const trimmed = url.trim();
    if (!trimmed) throw new Error("empty URL");

    // 0.3.0: проверка дубликатов — сравниваем полный URL точно.
    // Если уже есть такая же подписка, ничего не добавляем и показываем
    // юзеру toast (возвращаем id существующей для callsite).
    const dup = get().subscriptions.find((s) => s.url === trimmed);
    if (dup) {
      showToast({
        kind: "warning",
        title: i18n.t("toast.subscriptionDuplicate.title"),
        message: i18n.t("toast.subscriptionDuplicate.message"),
        durationMs: 4000,
      });
      return dup.id;
    }
    // Также проверяем legacy URL (на случай если subscriptions[] ещё не
    // мигрирован — юзер пытается добавить тот же URL что в legacy).
    if (get().url === trimmed && get().subscriptions.length === 0) {
      showToast({
        kind: "warning",
        title: i18n.t("toast.subscriptionDuplicate.title"),
        message: i18n.t("toast.subscriptionDuplicate.message"),
        durationMs: 4000,
      });
      // Возвращаем placeholder id; при следующем fetchSubscription
      // legacy замигрируется в subscriptions[0] и duplicate-check
      // сработает в обычном виде.
      return "__legacy_dup__";
    }

    // 0.3.0: если subscriptions пусты, но legacy URL уже есть (юзер
    // ещё не делал hard-reload после миграции 0.3.0), сначала
    // мигрируем legacy в subscriptions[0]. Без этого новая подписка
    // становится primary и legacy URL «теряется» (его нет в массиве).
    let existing = get().subscriptions;
    if (existing.length === 0) {
      const legacyUrl = get().url;
      const legacyHwid = get().hwid;
      if (legacyUrl.trim()) {
        const legacyId = genId();
        const legacySub: Subscription = {
          id: legacyId,
          url: legacyUrl,
          hwid: legacyHwid,
          meta: get().meta,
          lastFetchedAt: get().lastFetchedAt,
          loading: false,
          error: null,
          engineOverride: null,
          servers: [],
          pings: [],
        };
        await keyringSet(`${URL_KEYRING}:${legacyId}`, legacyUrl);
        if (legacyHwid.trim()) {
          await keyringSet(`${HWID_KEYRING}:${legacyId}`, legacyHwid);
        }
        existing = [legacySub];
        set({ subscriptions: existing, primaryId: legacyId });
        saveSubsIndex([legacyId]);
      }
    }

    const id = genId();
    const sub = newSubscription(trimmed, "");
    sub.id = id;
    await keyringSet(`${URL_KEYRING}:${id}`, trimmed);
    const next = [...existing, sub];
    set({ subscriptions: next });
    // Если это вообще первая подписка (и legacy URL не было) —
    // становится primary и legacy url синхронизируется.
    if (existing.length === 0) {
      set({ primaryId: id, url: trimmed });
      await keyringSet(URL_KEYRING, trimmed);
    }
    saveSubsIndex(next.map((s) => s.id));
    // 0.3.0 Этап 6: fetch для любой подписки. Primary использует legacy
    // fetchSubscription (синкнутся state.servers/meta для backward compat),
    // non-primary — fetchSubscriptionById (хранит результаты только в
    // sub'а; legacy state не трогается).
    if (get().primaryId === id) {
      await get().fetchSubscription();
    } else {
      await get().fetchSubscriptionById(id);
    }
    return id;
  },

  async removeSubscription(id) {
    const subs = get().subscriptions;
    const sub = subs.find((s) => s.id === id);
    if (!sub) return;
    const wasPrimary = get().primaryId === id;

    // Удаляем keyring entries (и legacy keys если был primary).
    await Promise.all([
      keyringDelete(`${URL_KEYRING}:${id}`),
      keyringDelete(`${HWID_KEYRING}:${id}`),
    ]);

    const remaining = subs.filter((s) => s.id !== id);
    set({ subscriptions: remaining });
    saveSubsIndex(remaining.map((s) => s.id));

    if (wasPrimary) {
      // Promote first remaining to primary, или если пусто — полная очистка.
      if (remaining.length > 0) {
        const next = remaining[0];
        set({ primaryId: next.id, url: next.url, hwid: next.hwid });
        await keyringSet(URL_KEYRING, next.url);
        if (next.hwid.trim()) await keyringSet(HWID_KEYRING, next.hwid);
        else await keyringDelete(HWID_KEYRING);
        // Fetch новой primary (servers нужно перезагрузить).
        await get().fetchSubscription();
      } else {
        // Это была последняя подписка — выкидываем все legacy данные.
        await get().deleteSubscription();
      }
    }
  },

  setPrimaryId(id) {
    const sub = get().subscriptions.find((s) => s.id === id);
    if (!sub) return;
    set({ primaryId: id, url: sub.url, hwid: sub.hwid, meta: sub.meta });
    // Sync legacy keyring keys на новый primary.
    void keyringSet(URL_KEYRING, sub.url);
    if (sub.hwid.trim()) void keyringSet(HWID_KEYRING, sub.hwid);
    else void keyringDelete(HWID_KEYRING);
  },

  setEngineOverride(id, engine) {
    set({
      subscriptions: get().subscriptions.map((s) =>
        s.id === id ? { ...s, engineOverride: engine } : s
      ),
    });
  },

  getEffectiveEngine(id) {
    const sub = get().subscriptions.find((s) => s.id === id);
    if (!sub) return useSettingsStore.getState().engine;
    // Приоритет: per-subscription override → header X-Nemefisto-Engine
    // → settings.engine (default fallback). Header «xray» нормализуется
    // в «sing-box» (после миграции 0.1.2 xray = sing-box семантически).
    if (sub.engineOverride) return sub.engineOverride;
    const headerEngine = sub.meta?.engine;
    if (headerEngine === "mihomo") return "mihomo";
    if (headerEngine === "sing-box" || headerEngine === "xray") return "sing-box";
    return useSettingsStore.getState().engine;
  },

  async fetchSubscriptionById(id) {
    const sub = get().subscriptions.find((s) => s.id === id);
    if (!sub) return;
    if (!sub.url.trim()) return;
    // Помечаем sub как loading для UI.
    set({
      subscriptions: get().subscriptions.map((s) =>
        s.id === id ? { ...s, loading: true, error: null } : s
      ),
    });
    const settings = useSettingsStore.getState();
    const ua = effectiveUserAgent(
      get().getEffectiveEngine(id),
      settings.userAgent,
      settings.userAgentTouched
    );
    try {
      const result = await invoke<FetchSubscriptionRaw>("fetch_subscription", {
        url: sub.url,
        hwidOverride: sub.hwid.trim() || null,
        userAgent: ua.trim() || null,
        sendHwid: settings.sendHwid,
      });
      const now = Date.now();
      const normalized = normalizeMeta(result.meta);
      // Tag servers с subscriptionId.
      const tagged = result.servers.map((s) => ({ ...s, subscriptionId: id }));
      // Update этой sub's data.
      set({
        subscriptions: get().subscriptions.map((s) =>
          s.id === id
            ? {
                ...s,
                meta: normalized,
                lastFetchedAt: now,
                servers: tagged,
                pings: [],
                loading: false,
                error: null,
              }
            : s
        ),
      });
      // Если primary — синхронизируем legacy state для backward compat
      // (компоненты вроде vpnStore.connect и старого ServerSelector).
      if (get().primaryId === id) {
        saveToStorage(LAST_FETCH_KEY, String(now));
        set({
          servers: tagged,
          meta: normalized,
          pings: [],
          lastFetchedAt: now,
          loading: false,
        });
        // Restore selectedIndex по сохранённому имени (как и в legacy fetch).
        const restoredIndex = findSelectedIndexByName(tagged);
        if (restoredIndex >= 0) {
          useVpnStore.setState({ selectedIndex: restoredIndex });
        }
      }
      // Авто-пинг для этой sub.
      void get().pingAllOf(id);
    } catch (e) {
      set({
        subscriptions: get().subscriptions.map((s) =>
          s.id === id ? { ...s, loading: false, error: String(e) } : s
        ),
      });
    }
  },

  async pingAllOf(id) {
    const sub = get().subscriptions.find((s) => s.id === id);
    if (!sub || sub.servers.length === 0) return;
    // Текущая Rust команда `ping_servers` пингует state.servers (singleton).
    // Чтобы пингануть servers конкретной подписки, нужно временно
    // подменить state.servers через Rust set_servers — командной нет, и
    // делать её сейчас переусложнение. Простой workaround: если sub
    // primary, ping_servers даёт корректный результат; иначе используем
    // primary's pings (заглушка, обновятся при swap primary).
    if (get().primaryId !== id) return;
    set({ pingsLoading: true });
    try {
      const result = await invoke<(number | null)[]>("ping_servers");
      set({ pings: result, pingsLoading: false });
      set({
        subscriptions: get().subscriptions.map((s) =>
          s.id === id ? { ...s, pings: result } : s
        ),
      });
    } catch {
      set({ pingsLoading: false });
    }
  },

  // ─── Legacy API (sync'ится с primary subscription) ───────────────────

  setUrl: (url) => {
    set({ url });
    // 0.3.0: обновляем primary subscription тоже (sync legacy ↔ multi).
    const primaryId = get().primaryId;
    if (primaryId) {
      set({
        subscriptions: get().subscriptions.map((s) =>
          s.id === primaryId ? { ...s, url } : s
        ),
      });
      void keyringSet(`${URL_KEYRING}:${primaryId}`, url);
    }
    // Чувствительные значения пишем в Windows Credential Manager.
    // localStorage больше НЕ используется как источник правды — оставляем
    // пустым, чтобы старые версии приложения не подсунули устаревший URL.
    saveToStorage(URL_KEY, "");
    void keyringSet(URL_KEYRING, url);
  },
  setHwid: (hwid) => {
    set({ hwid });
    // 0.3.0: sync с primary subscription тоже.
    const primaryId = get().primaryId;
    if (primaryId) {
      set({
        subscriptions: get().subscriptions.map((s) =>
          s.id === primaryId ? { ...s, hwid } : s
        ),
      });
      if (hwid.trim()) void keyringSet(`${HWID_KEYRING}:${primaryId}`, hwid);
      else void keyringDelete(`${HWID_KEYRING}:${primaryId}`);
    }
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

    // 0.3.0 multi-subscription bootstrap. Сценарии:
    //   A) localStorage SUBS_INDEX_KEY есть → читаем все по списку из
    //      keyring `subscription_url:${id}` / `hwid_override:${id}`.
    //   B) Списка нет, но legacy URL_KEYRING есть (юзер обновился с
    //      0.2.x) → создаём subscriptions[0] из этого URL, сохраняем
    //      его и в новые per-id ключи (миграция). Legacy URL_KEYRING
    //      продолжает синхронизироваться с primary для backward compat.
    //   C) Всё пусто → subscriptions = [], primaryId = null. Welcome.
    const ids = loadSubsIndex();
    if (ids.length > 0) {
      // Сценарий A
      const subs: Subscription[] = [];
      for (const id of ids) {
        const u = await keyringGet(`${URL_KEYRING}:${id}`);
        const h = await keyringGet(`${HWID_KEYRING}:${id}`);
        if (!u) continue; // потерянная запись — пропускаем (cleanup ниже)
        subs.push({
          id,
          url: u,
          hwid: h,
          meta: null,
          lastFetchedAt: null,
          loading: false,
          error: null,
          engineOverride: null,
          servers: [],
          pings: [],
        });
      }
      // Если из-за потерянных записей итог отличается — обновляем индекс.
      if (subs.length !== ids.length) saveSubsIndex(subs.map((s) => s.id));
      if (subs.length > 0) {
        set({ subscriptions: subs, primaryId: subs[0].id });
        // Legacy fields синхронизируются с primary (для callsites,
        // которые ещё читают `url`/`hwid`/`meta`).
        set({ url: subs[0].url, hwid: subs[0].hwid });
      }
    } else if (urlFromKeyring) {
      // Сценарий B — миграция legacy single-sub в multi.
      const id = genId();
      const sub: Subscription = {
        id,
        url: urlFromKeyring,
        hwid: hwidFromKeyring || "",
        meta: null,
        lastFetchedAt: get().lastFetchedAt,
        loading: false,
        error: null,
        engineOverride: null,
        servers: [],
        pings: [],
      };
      // Сохраняем в новые per-id ключи. Legacy URL_KEYRING остаётся.
      await keyringSet(`${URL_KEYRING}:${id}`, urlFromKeyring);
      if (hwidFromKeyring) {
        await keyringSet(`${HWID_KEYRING}:${id}`, hwidFromKeyring);
      }
      set({ subscriptions: [sub], primaryId: id });
      saveSubsIndex([id]);
    }
  },

  async fetchSubscription() {
    const { url, hwid } = get();
    if (!url.trim()) return;

    // 0.3.0 auto-bootstrap: если subscriptions[] пуст, создаём primary
    // из текущего legacy URL ДО fetch'а. Иначе после fetch'а subscriptions[]
    // останется пустым (т.к. fetchSubscription не дёргает addSubscription),
    // и UI карточек подписок не покажется. Это случай:
    // - Welcome → setUrl(url) → fetchSubscription() (старый flow)
    // - юзер запустил app до миграции (legacy single-sub)
    // - keyring пуст, но Rust state имеет cached servers
    if (get().subscriptions.length === 0) {
      const bootstrapId = genId();
      const bootstrapSub: Subscription = {
        id: bootstrapId,
        url,
        hwid,
        meta: get().meta,
        lastFetchedAt: get().lastFetchedAt,
        loading: false,
        error: null,
        engineOverride: null,
        servers: [],
        pings: [],
      };
      set({ subscriptions: [bootstrapSub], primaryId: bootstrapId });
      saveSubsIndex([bootstrapId]);
      // Сохраняем в новые per-id ключи (legacy URL_KEYRING продолжает
      // синхронизироваться через setUrl).
      await keyringSet(`${URL_KEYRING}:${bootstrapId}`, url);
      if (hwid.trim()) {
        await keyringSet(`${HWID_KEYRING}:${bootstrapId}`, hwid);
      }
    }

    const settings = useSettingsStore.getState();
    // 8.B / 0.3.0: эффективный UA зависит от движка. Когда есть primary
    // подписка с engineOverride или header X-Nemefisto-Engine — берём
    // её engine, иначе settings.engine. Per-subscription UA для
    // вторичных подписок реализуется в Этапе 3 через fetchSubscriptionById.
    const primaryId = get().primaryId;
    const effectiveEngine = primaryId
      ? get().getEffectiveEngine(primaryId)
      : settings.engine;
    const ua = effectiveUserAgent(
      effectiveEngine,
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
      // 0.3.0: tag servers с subscriptionId для multi-source группировки
      // и engine resolution. Все servers первой fetch-итерации
      // принадлежат primary subscription.
      const tagged = primaryId
        ? result.servers.map((s) => ({ ...s, subscriptionId: primaryId }))
        : result.servers;
      set({
        servers: tagged,
        meta: normalized,
        pings: [],
        lastFetchedAt: now,
        loading: false,
      });
      // 0.3.0: sync servers/meta/lastFetchedAt в primary subscription.
      if (primaryId) {
        set({
          subscriptions: get().subscriptions.map((s) =>
            s.id === primaryId
              ? {
                  ...s,
                  servers: tagged,
                  meta: normalized,
                  lastFetchedAt: now,
                  pings: [],
                  loading: false,
                }
              : s
          ),
        });
      }
      // 0.2.4: восстанавливаем выбранный сервер ПО ИМЕНИ. После refetch
      // массив пересоздаётся, индексы сбиваются — поэтому ищем по
      // (subscriptionId, name) паре. 0.3.0: toast «server gone» удалён —
      // в multi-subscription при swap'е подписок имена естественно не
      // совпадают, это не потеря, а просто другой источник. Юзер просто
      // увидит unselected state и выберет новый сервер сам.
      {
        const restoredIndex = findSelectedIndexByName(result.servers);
        if (restoredIndex >= 0) {
          useVpnStore.setState({ selectedIndex: restoredIndex });
        } else if (result.servers.length === 1) {
          // Auto-select для mihomo-passthrough single-entry — без него
          // MihomoGroupsInline не отрисуется (нужен selectedServer).
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
        // 0.3.0 auto-bootstrap: если subscriptions[] пуст НО Rust имеет
        // cached servers — создаём fallback subscription из legacy URL.
        // Это покрывает случаи когда loadSecureCreds B-сценарий не
        // отработал (URL_KEYRING пуст), но юзер всё ещё имеет servers
        // в Rust runtime state.
        let primaryId = get().primaryId;
        if (!primaryId && get().subscriptions.length === 0) {
          const url = get().url;
          if (url.trim()) {
            const id = genId();
            const sub: Subscription = {
              id,
              url,
              hwid: get().hwid,
              meta: get().meta,
              lastFetchedAt: get().lastFetchedAt,
              loading: false,
              error: null,
              engineOverride: null,
              servers: [],
              pings: [],
            };
            set({ subscriptions: [sub], primaryId: id });
            saveSubsIndex([id]);
            await keyringSet(`${URL_KEYRING}:${id}`, url);
            if (sub.hwid.trim()) {
              await keyringSet(`${HWID_KEYRING}:${id}`, sub.hwid);
            }
            primaryId = id;
          } else {
            // URL пуст и subscriptions[] пуст — Rust state хранит orphan
            // servers от предыдущего runtime'а (например после полного
            // deleteSubscription в прошлой сессии). Юзер не может ими
            // управлять (нет URL для refresh, нет ⋯ меню). Очищаем —
            // App.tsx покажет Welcome с возможностью ввести новый URL.
            set({ servers: [], meta: null, pings: [] });
            return;
          }
        }
        const tagged = primaryId
          ? servers.map((s) => ({ ...s, subscriptionId: primaryId! }))
          : servers;
        set({ servers: tagged });
        if (primaryId) {
          set({
            subscriptions: get().subscriptions.map((s) =>
              s.id === primaryId ? { ...s, servers: tagged } : s
            ),
          });
        }
        // 0.2.4: на старте app vpnStore.selectedIndex = null (он
        // живёт только в памяти), так что восстанавливаем по имени из
        // localStorage. Auto-select 0 для одиночного сервера —
        // важно для mihomo-passthrough.
        const restoredIndex = findSelectedIndexByName(servers);
        if (restoredIndex >= 0) {
          useVpnStore.setState({ selectedIndex: restoredIndex });
        } else if (servers.length === 1) {
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

  async deleteSubscription() {
    // 0.2.4: полное удаление подписки. Если VPN активен — сначала
    // тушим (без него выбранный сервер «висит» в ядре, но в UI его
    // уже нет). После очистки экран должен вернуться к Welcome.
    const vpn = useVpnStore.getState();
    if (vpn.status === "running") {
      try {
        await vpn.disconnect();
      } catch {
        // продолжаем удаление — даже если disconnect упал, чистка
        // важнее (state восстановится сам через refresh).
      }
    }

    // Кеш серверов в Rust живёт только в памяти SubscriptionState
    // (Mutex'ы), без диска. После закрытия app state сбрасывается;
    // в текущей сессии оставшийся в Rust список не критичен — UI
    // показывает Welcome потому что мы обнулили `servers` ниже.

    // 1. Удаляем URL и override-HWID из keyring (legacy + per-id для
     //    каждой подписки в multi-state).
    await Promise.all([
      keyringDelete(URL_KEYRING),
      keyringDelete(HWID_KEYRING),
      ...get().subscriptions.flatMap((s) => [
        keyringDelete(`${URL_KEYRING}:${s.id}`),
        keyringDelete(`${HWID_KEYRING}:${s.id}`),
      ]),
    ]);

    // 2. Чистим persisted selectedIndex, last-fetched timestamp и
     //    multi-subscription index.
    try {
      localStorage.removeItem(LAST_FETCH_KEY);
      localStorage.removeItem("nemefisto.selectedServerName.v1");
      localStorage.removeItem(SUBS_INDEX_KEY);
    } catch {
      // приватный режим — игнорируем
    }

    // 3. Сбрасываем in-memory state (multi + legacy) и selectedIndex в vpnStore.
    set({
      subscriptions: [],
      primaryId: null,
      servers: [],
      meta: null,
      pings: [],
      lastFetchedAt: null,
      url: "",
      hwid: "",
      error: null,
    });
    useVpnStore.setState({ selectedIndex: null });

    showToast({
      kind: "success",
      title: i18n.t("toast.subscriptionDeleted.title"),
      message: i18n.t("toast.subscriptionDeleted.message"),
      durationMs: 4000,
    });
  },
}));
