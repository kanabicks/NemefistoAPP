import { create } from "zustand";

export type SortMode = "none" | "ping" | "name";
export type Theme = "dark" | "light" | "midnight" | "sunset" | "sand";
export type Background = "crystal" | "tunnel" | "globe" | "particles";
export type ButtonStyle = "glass" | "flat" | "neon" | "metallic";

/**
 * VPN-движок. Выбирается per-session, не миксуется.
 * - **sing-box** (default с 0.1.2 — заменил xray) — современный движок
 *   с built-in TUN (gVisor stack), нативной поддержкой
 *   vless+REALITY/Vision, hy2, tuic, wireguard, и встроенным
 *   anti-DPI (`tls.fragment`, server-resolve через DoH). Подписка
 *   запрашивается с UA `Happ/2.7.0` — Marzban-панели отдают xray-JSON
 *   (мы конвертируем), Remnawave — sing-box JSON (passthrough).
 * - **mihomo** — Clash Meta форк. AnyTLS, Mieru, native PROCESS-NAME
 *   routing. Используется когда сервер из подписки `engine_compat`
 *   содержит "mihomo" либо пользователь явно выбрал.
 */
export type Engine = "sing-box" | "mihomo";

/**
 * Правило per-process routing (этап 8.D). Применяется только в Mihomo
 * через нативный matcher `PROCESS-NAME` + `find-process-mode: always`.
 * В Xray такие правила работать не умеют (на Windows нет нативной
 * поддержки) — UI показывает баннер «работает только в Mihomo».
 *
 * - **exe** — имя исполняемого файла без пути, в нижнем регистре
 *   (Mihomo сравнивает case-insensitive). Например `telegram.exe`.
 * - **action** — куда направить трафик процесса:
 *   - `proxy` — через VPN (даже если по другим правилам шёл бы direct);
 *   - `direct` — мимо VPN (даже если по гео-правилам шёл бы через proxy);
 *   - `block` — REJECT (соединение разрывается, процесс не имеет сети).
 * - **comment** — необязательная заметка пользователя для UI.
 */
export type AppRuleAction = "proxy" | "direct" | "block";
export type AppRule = {
  exe: string;
  action: AppRuleAction;
  comment?: string;
};

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

  /** TUN-only «strict mode» (этап 13.R). Если on — proxy-режим скрыт
   *  из UI, остаётся только TUN. Параноидальная опция: пользователь
   *  не хочет иметь живой SOCKS5 на loopback (даже на рандомном
   *  порту) и предпочитает чтобы трафик шёл строго через WinTUN
   *  адаптер. При активации — если текущий режим proxy, авто-переключаем
   *  на tun. Default off (proxy быстрее стартует и по умолчанию
   *  привычнее пользователям). */
  tunOnlyStrict: boolean;

  /** Kill switch (этап 13.D — WFP). Если on — при connect helper-сервис
   *  ставит фильтры в Windows Filtering Platform на уровне ядра:
   *  block-all + allowlist (loopback, опц. LAN, IP VPN-сервера, наши
   *  процессы по app-id). DYNAMIC session: фильтры **автоматически
   *  удаляются** если helper-процесс упал — пользователь не остаётся
   *  без интернета. Дополнительная страховка — cleanup orphan-фильтров
   *  на старте helper'а. */
  killSwitch: boolean;

  /** Strict-mode kill-switch (этап 13.S). Если on — даже сам xray/mihomo
   *  не имеет общего outbound-allow, только на server_ips. Блокирует
   *  direct-маршруты xray (например `geosite:ru → DIRECT`). Закрывает
   *  кейс «kill-switch on = ничего не идёт мимо VPN». ⚠️ Несовместим
   *  со split-routing конфигами где RU-сайты идут direct — они
   *  перестанут открываться. Default off для совместимости с типовыми
   *  ожиданиями (Mullvad/Nord/Proton-семантика). */
  killSwitchStrict: boolean;

  /** 13.Q: если активного routing-профиля нет — применять встроенный
   *  «минимальный RU» шаблон (geosite:ru → DIRECT, geoip:ru → DIRECT,
   *  geosite:category-ads-all → BLOCK). Default off для совместимости —
   *  пользователи которые не хотят split-routing не увидят неожиданного
   *  поведения. */
  autoApplyMinimalRuRules: boolean;

  /** DNS leak protection (этап 13.D step B). Если on — при активном
   *  kill-switch блокируется весь :53/UDP+TCP кроме нашего VPN-DNS.
   *  Защита от приложений которые делают DNS-запросы мимо VPN. ⚠️ В
   *  proxy-режиме может ломать приложения с собственным системным
   *  DNS — лучше использовать в TUN-режиме. */
  dnsLeakProtection: boolean;

  /** Активный VPN-движок (этап 8.B). См. тип `Engine` выше. */
  engine: Engine;
  /** Override-флаг для server-driven UX. Если false — заголовок
   *  `X-Nemefisto-Engine` подписки имеет приоритет над юзер-выбором. */
  engineTouched: boolean;

  /** Touched-флаг для userAgent. Если false — `effectiveUserAgent`
   *  возвращает зависящий от движка дефолт (Happ/* для Xray —
   *  Marzban-style xray-json; clash-verge для Mihomo — clash YAML).
   *  Когда пользователь явно правит поле в UI → флаг ставится в true,
   *  и используется ровно то значение что вписано. */
  userAgentTouched: boolean;

  /** Правила per-process routing (этап 8.D). Применяются только в
   *  Mihomo. См. тип `AppRule` выше. Пустой массив — no-op. */
  appRules: AppRule[];

  // ── Глобальные горячие клавиши (этап 13.N) ─────────────────────────
  /** Accelerator (`Ctrl+Shift+V` и т.п.) для toggle VPN. `null` —
   *  не зарегистрирована. Tauri-формат: `Modifier+...+Key`, поддержка
   *  `CommandOrControl`, `Ctrl`, `Alt`, `Shift`, `Super`. */
  shortcutToggleVpn: string | null;
  /** Accelerator для show/hide главного окна (как клик по трею). */
  shortcutShowHide: string | null;
  /** Accelerator для переключения proxy ↔ TUN режима. */
  shortcutSwitchMode: string | null;

  /** Плавающее мини-окно (этап 13.O). Если on — отдельное окошко
   *  поверх всех с status-dot и live-скоростью передачи данных.
   *  Состояние применяется при старте приложения и при каждом
   *  toggle: фронт зовёт `show_floating_window` / `hide_floating_window`. */
  floatingWindow: boolean;

  /** Авто-проверка IP/DNS после успешного connect (этап 13.B/13.H).
   *  ipapi.co + Cloudflare DoH whoami.cloudflare за один тост.
   *  Сетевые запросы платные по latency (~1-2 сек), кому-то может
   *  не нравиться — toggle. По дефолту on. */
  autoLeakTest: boolean;

  // ── Доверенные Wi-Fi сети (этап 13.M) ──────────────────────────────
  /** Список SSID которые считаются «домашними» — там VPN автоматически
   *  выключается (если включён `trustedSsidAction === "disconnect"`).
   *  Сравнение точное и case-sensitive (Windows такие и хранит). */
  trustedSsids: string[];
  /** Что делать при подключении к Wi-Fi из `trustedSsids`:
   *  - `ignore` (default) — ничего;
   *  - `disconnect` — отключить активный VPN, флаг
   *    `autoDisconnectedBySsid` помечается чтобы при выходе из сети
   *    переподключиться обратно (если включено `autoConnectOnLeave`). */
  trustedSsidAction: "ignore" | "disconnect";
  /** Автоподключение когда уходим с доверенной сети обратно в
   *  обычную. Срабатывает только если VPN был отключён нами же
   *  по trusted-правилу (а не самим пользователем). */
  autoConnectOnLeave: boolean;

  /** Предпочитаемая нода в proxy-группах mihomo (8.F).
   *  Запоминается между сессиями: пользователь до connect выбрал
   *  «Latvia» в группе `→ Nemefisto VPN` — после connect мы
   *  автоматически переключаем эту группу на Latvia через external-
   *  controller. Записи скапливаются по разным группам — у каждой
   *  своя предпочитаемая нода. `null` значение в map не используется
   *  (отсутствие ключа = нет преференса).
   *
   *  Имена групп берутся из YAML подписки и нестабильны между
   *  провайдерами — при смене подписки старые ключи будут невалидны
   *  и просто проигнорируются. */
  preferredMihomoNodes: Record<string, string>;
};

/**
 * UA по умолчанию для Xray-движка.
 *
 * Большинство панелей подписок (Marzban, 3x-ui, sing-box-panel)
 * детектят `Happ/*` UA и отдают полный Xray JSON-конфиг с готовыми
 * routing-правилами (RU-сайты direct, ads block и т.д.). Для Xray это
 * то что надо.
 */
/**
 * UA по умолчанию для sing-box-движка (default с 0.1.2).
 *
 * `Happ/2.7.0` — стандартный UA современных VPN-клиентов. Marzban-панели
 * по этому UA отдают полный Xray JSON-конфиг (мы конвертируем его в
 * sing-box JSON через `convert_xray_json_to_singbox`). Remnawave-панели
 * по этому же UA отдают **sing-box JSON** напрямую — мы используем его
 * через passthrough (`patch_singbox_json`).
 */
export const DEFAULT_USER_AGENT_SINGBOX = "Happ/2.7.0";

/**
 * UA по умолчанию для Mihomo-движка.
 *
 * Те же панели на `clash-verge/*` UA отдают clash YAML с такими же
 * routing-группами, но в формате Mihomo. Это позволяет владельцу
 * подписки тонко настраивать конфиги под каждое ядро отдельно.
 */
export const DEFAULT_USER_AGENT_MIHOMO = "clash-verge/v2.0.0";

/** Backwards-compat alias — используется в местах где UA всё ещё
 *  «общий». Equivalent to `DEFAULT_USER_AGENT_XRAY`. */
export const DEFAULT_USER_AGENT = DEFAULT_USER_AGENT_SINGBOX;

/**
 * Эффективный UA для запроса подписки. Если пользователь явно правил
 * поле (`userAgentTouched`) — возвращаем его значение «как есть»,
 * иначе — дефолт под выбранный движок.
 */
export function effectiveUserAgent(
  engine: Engine,
  userAgent: string,
  userAgentTouched: boolean
): string {
  if (userAgentTouched) return userAgent;
  return engine === "mihomo" ? DEFAULT_USER_AGENT_MIHOMO : DEFAULT_USER_AGENT_SINGBOX;
}

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
  tunOnlyStrict: false,
  killSwitch: false,
  killSwitchStrict: false,
  autoApplyMinimalRuRules: false,
  dnsLeakProtection: false,
  engine: "sing-box",
  engineTouched: false,
  userAgentTouched: false,
  appRules: [],
  shortcutToggleVpn: "Ctrl+Shift+V",
  shortcutShowHide: "Ctrl+Shift+M",
  shortcutSwitchMode: null,
  floatingWindow: false,
  autoLeakTest: true,
  trustedSsids: [],
  trustedSsidAction: "ignore",
  autoConnectOnLeave: false,
  preferredMihomoNodes: {},
};

const KEY = "nemefisto.settings.v1";

const load = (): Settings => {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULTS;
    const parsed = JSON.parse(raw) as Partial<Settings>;
    const merged: Settings = { ...DEFAULTS, ...parsed };
    // sing-box миграция (0.1.2): "xray" → "sing-box". Старые конфиги
    // должны автоматически переехать на новый default-движок без
    // сбрасывания engineTouched (уважаем что пользователь раньше
    // явно выбрал не-Mihomo).
    if ((merged.engine as unknown as string) === "xray") {
      merged.engine = "sing-box";
    }
    return merged;
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
    if (key === "engine") next.engineTouched = true;
    if (key === "userAgent") next.userAgentTouched = true;
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
