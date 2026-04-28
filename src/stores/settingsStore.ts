import { create } from "zustand";

export type SortMode = "none" | "ping" | "name";
export type Theme = "dark" | "light" | "midnight" | "sunset";

export type Settings = {
  /** Авто-обновление подписки */
  autoRefresh: boolean;
  /** Интервал авто-обновления в часах */
  autoRefreshHours: number;
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
  /** Тема оформления (тёмная по умолчанию). */
  theme: Theme;
};

export const DEFAULT_USER_AGENT = "Happ/2.7.0";

const DEFAULTS: Settings = {
  autoRefresh: false,
  autoRefreshHours: 1,
  refreshOnOpen: false,
  pingOnOpen: true,
  connectOnOpen: false,
  sendHwid: true,
  userAgent: DEFAULT_USER_AGENT,
  sort: "none",
  allowLan: false,
  theme: "dark",
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
    const next = { ...get(), [key]: value };
    save(next);
    setState({ [key]: value } as Pick<Settings, typeof key>);
  },
  reset: () => {
    save(DEFAULTS);
    setState(DEFAULTS);
  },
}));
