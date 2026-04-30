import { create } from "zustand";

export type SortMode = "none" | "ping" | "name";
export type Theme = "dark" | "light" | "midnight" | "sunset" | "sand";
export type Background = "crystal" | "tunnel" | "globe" | "particles";
export type ButtonStyle = "glass" | "flat" | "neon" | "metallic";

/**
 * Готовые «темы-пресеты» — отдельная ось настройки, не комбинация
 * существующих theme/background/buttonStyle. У каждого пресета своя
 * уникальная палитра (CSS-переменные через `data-preset` на <html>),
 * фиксированный 3D-фон и стиль кнопки.
 *
 * Когда выбран любой пресет кроме `none`, обычные селекты темы/фона/
 * стиля становятся недоступны (управляется пресетом).
 *
 * Названия — нейтральные ассоциации:
 *  - fluent    — стиль Microsoft Fluent (acrylic, синий акцент)
 *  - cupertino — стиль Apple (мягкие пастельные, минималистичные)
 *  - vice      — стиль 80s neon arcade (фуксия + cyan)
 *  - arcade    — игровая консоль (зелёный неон по тёмному)
 *  - glacier   — холодное матовое стекло, лёд
 */
export type Preset = "none" | "fluent" | "cupertino" | "vice" | "arcade" | "glacier";

/** Какой 3D-фон рендерить при активном пресете. */
export const PRESET_BACKGROUND: Record<Preset, Background> = {
  none:      "crystal",   // не используется (preset === none → берём из settings.background)
  fluent:    "globe",
  cupertino: "particles",
  vice:      "tunnel",
  arcade:    "crystal",
  glacier:   "particles",
};

/** Какой стиль кнопки применять при активном пресете. */
export const PRESET_BUTTON_STYLE: Record<Preset, ButtonStyle> = {
  none:      "glass",     // не используется
  fluent:    "glass",
  cupertino: "flat",
  vice:      "neon",
  arcade:    "neon",
  glacier:   "glass",
};

/**
 * Палитра кристалла/линий/частиц в Three.js per-preset. Используется
 * Scene3D вместо theme-палитры когда preset активен.
 */
export const PRESET_SCENE_PALETTE: Record<
  Exclude<Preset, "none">,
  { base: number; dim: number; solid: number; fog: number }
> = {
  fluent:    { base: 0x60a5fa, dim: 0x3b6cb0, solid: 0x14203a, fog: 0x0c1424 },
  cupertino: { base: 0xff375f, dim: 0xb87080, solid: 0xeae0e0, fog: 0xf0e7e7 },
  vice:      { base: 0xff37a8, dim: 0xa02370, solid: 0x2a0a3a, fog: 0x1a0a3a },
  arcade:    { base: 0x5cc62a, dim: 0x3a7a1f, solid: 0x1a261a, fog: 0x0a140a },
  glacier:   { base: 0x9bc9f0, dim: 0x5a8bb8, solid: 0x14222e, fog: 0x0c1827 },
};

export type Settings = {
  /** Авто-обновление подписки */
  autoRefresh: boolean;
  /** Интервал авто-обновления в часах */
  autoRefreshHours: number;
  /** Флаг "пользователь явно менял интервал". Если false — заголовок
   *  `profile-update-interval` подписки имеет приоритет. См. override-
   *  логику в плане 8.C. Сбрасывается через reset(). */
  autoRefreshHoursTouched: boolean;
  /** Обновлять подписку при запуске приложения */
  refreshOnOpen: boolean;
  /** Запускать пинг всех серверов при запуске */
  pingOnOpen: boolean;
  /** Авто-подключение к последнему выбранному серверу при запуске */
  connectOnOpen: boolean;
  /** Передавать HWID в заголовке x-hwid */
  sendHwid: boolean;
  /** User-Agent для HTTP-запроса подписки */
  userAgent: string;
  /** Сортировка серверов в списке */
  sort: SortMode;
  /** Разрешить подключения из LAN (inbound listen 0.0.0.0) */
  allowLan: boolean;
  /** Тема оформления (используется когда preset === "none"). */
  theme: Theme;
  /** Тип 3D-фона (используется когда preset === "none"). */
  background: Background;
  /** Стиль главной кнопки (используется когда preset === "none"). */
  buttonStyle: ButtonStyle;
  /**
   * Активный пресет. `none` — пользователь сам подбирает theme/bg/style.
   * Любое другое значение переопределяет всё разом.
   */
  preset: Preset;
  /** Override-флаги для server-driven UX (8.C, X-Nemefisto-*). Если
   *  false — соответствующее значение из заголовка подписки имеет
   *  приоритет над юзер-настройкой. Сбрасываются через reset(). */
  themeTouched: boolean;
  backgroundTouched: boolean;
  buttonStyleTouched: boolean;
  presetTouched: boolean;

  // ── Anti-DPI (этап 10) ──────────────────────────────────────────────
  /** TCP-фрагментация: режет TLS ClientHello (или другие пакеты) на
   *  куски, мешая DPI собрать его. Реализовано через freedom-outbound
   *  Xray с настройкой `fragment`. */
  antiDpiFragmentation: boolean;
  /** Какие пакеты фрагментировать: `tlshello` (default — только
   *  TLS handshake), `1-3` (первые 1-3 пакета), `all` (все). */
  antiDpiFragmentationPackets: string;
  /** Длина одного фрагмента в байтах: формат `min-max`. */
  antiDpiFragmentationLength: string;
  /** Задержка между фрагментами в миллисекундах: `min-max`. */
  antiDpiFragmentationInterval: string;
  /** UDP шумовые пакеты — фейковые UDP-пакеты для запутывания DPI. */
  antiDpiNoises: boolean;
  /** Тип содержимого: `rand` (случайные байты), `str` (строка),
   *  `hex` (hex-строка). */
  antiDpiNoisesType: string;
  /** Содержимое пакета или его размер в формате `min-max`. */
  antiDpiNoisesPacket: string;
  /** Задержка между шумовыми пакетами `min-max` (мс). */
  antiDpiNoisesDelay: string;
  /** Резолвить адрес VPN-сервера через DoH (минуя системный DNS).
   *  Помогает при DNS-блокировках Роскомнадзора. */
  antiDpiServerResolve: boolean;
  /** DoH endpoint для резолва адреса сервера. */
  antiDpiResolveDoH: string;
  /** Bootstrap-IP для самого DoH-сервера (чтобы он сам не резолвился
   *  через себя). */
  antiDpiResolveBootstrap: string;
  /** Один общий override-флаг для всей anti-DPI секции. Если
   *  false — настройки из заголовков подписки `fragmentation-*` /
   *  `noises-*` / `server-address-resolve-*` имеют приоритет. */
  antiDpiTouched: boolean;

  /** Маскировка имени TUN-адаптера (этап 12.E). Если on — каждое
   *  подключение в TUN-режиме создаёт адаптер с нейтральным именем
   *  (wlan99 / Local Area Connection N / Ethernet N) вместо
   *  `nemefisto-<pid>`. Защита от детекта VPN приложениями типа
   *  МАХ/ВК/Госуслуг по `GetAdaptersAddresses`. */
  tunMasking: boolean;
};

export const DEFAULT_USER_AGENT = "Happ/2.7.0";

const DEFAULTS: Settings = {
  autoRefresh: false,
  autoRefreshHours: 1,
  autoRefreshHoursTouched: false,
  refreshOnOpen: false,
  pingOnOpen: true,
  connectOnOpen: false,
  sendHwid: true,
  userAgent: DEFAULT_USER_AGENT,
  sort: "none",
  allowLan: false,
  theme: "dark",
  background: "crystal",
  buttonStyle: "glass",
  preset: "none",
  themeTouched: false,
  backgroundTouched: false,
  buttonStyleTouched: false,
  presetTouched: false,

  // Anti-DPI: по дефолту всё выключено, разумные значения для случая
  // когда пользователь включит вручную.
  antiDpiFragmentation: false,
  antiDpiFragmentationPackets: "tlshello",
  antiDpiFragmentationLength: "10-20",
  antiDpiFragmentationInterval: "10-20",
  antiDpiNoises: false,
  antiDpiNoisesType: "rand",
  antiDpiNoisesPacket: "10-30",
  antiDpiNoisesDelay: "10-20",
  antiDpiServerResolve: false,
  antiDpiResolveDoH: "https://cloudflare-dns.com/dns-query",
  antiDpiResolveBootstrap: "1.1.1.1",
  antiDpiTouched: false,
  tunMasking: false,
};

const KEY = "nemefisto.settings.v1";

const load = (): Settings => {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULTS;
    const parsed = JSON.parse(raw) as Partial<Settings>;
    return { ...DEFAULTS, ...parsed };
  } catch {
    return DEFAULTS;
  }
};

const save = (s: Settings) => {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // приватный режим — игнорируем
  }
};

type Store = Settings & {
  set: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  reset: () => void;
};

export const useSettingsStore = create<Store>((setState, get) => ({
  ...load(),
  set: (key, value) => {
    const next: Settings = { ...get(), [key]: value };
    // Override-флаги: пользователь явно поменял настройку → перестаём
    // подхватывать значение из заголовка подписки. См. 8.C override-логику.
    if (key === "autoRefreshHours") next.autoRefreshHoursTouched = true;
    if (key === "theme") next.themeTouched = true;
    if (key === "background") next.backgroundTouched = true;
    if (key === "buttonStyle") next.buttonStyleTouched = true;
    if (key === "preset") next.presetTouched = true;
    // Любая правка anti-DPI поля → touched (override от заголовков
    // подписки больше не применяется).
    if (
      key === "antiDpiFragmentation" ||
      key === "antiDpiFragmentationPackets" ||
      key === "antiDpiFragmentationLength" ||
      key === "antiDpiFragmentationInterval" ||
      key === "antiDpiNoises" ||
      key === "antiDpiNoisesType" ||
      key === "antiDpiNoisesPacket" ||
      key === "antiDpiNoisesDelay" ||
      key === "antiDpiServerResolve" ||
      key === "antiDpiResolveDoH" ||
      key === "antiDpiResolveBootstrap"
    ) {
      next.antiDpiTouched = true;
    }
    save(next);
    setState(next);
  },
  reset: () => {
    save(DEFAULTS);
    setState(DEFAULTS);
  },
}));
