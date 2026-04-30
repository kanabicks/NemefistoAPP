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
    // Override-флаг: пользователь явно поменял интервал → перестаём
    // подхватывать значение из заголовка подписки.
    if (key === "autoRefreshHours") {
      next.autoRefreshHoursTouched = true;
    }
    save(next);
    setState(next);
  },
  reset: () => {
    save(DEFAULTS);
    setState(DEFAULTS);
  },
}));
