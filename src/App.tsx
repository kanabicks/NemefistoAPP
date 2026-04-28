import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";
import { useVpnStore, type VpnMode, type VpnStatus } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";
import {
  DEFAULT_USER_AGENT,
  useSettingsStore,
  type SortMode,
} from "./stores/settingsStore";
import { initDeepLinks } from "./lib/deepLinks";

const DASHBOARD_URL = "https://web.nemefisto.online";
const SUPPORT_URL = "https://t.me/nemefistovpn_bot";
const APP_VERSION = "0.1.0";

function openDashboard() {
  void openUrl(DASHBOARD_URL);
}
function openSupport() {
  void openUrl(SUPPORT_URL);
}

// ── Локализация ──────────────────────────────────────────────────────────────

const STATUS_PILL: Record<VpnStatus, { label: string; cls: string }> = {
  stopped: { label: "STANDBY", cls: "" },
  starting: { label: "ПОДКЛЮЧЕНИЕ", cls: "is-busy" },
  running: { label: "TUNNEL UP", cls: "is-running" },
  stopping: { label: "ОТКЛЮЧЕНИЕ", cls: "is-busy" },
  error: { label: "ERROR", cls: "is-error" },
};

const POWER_LABEL: Record<VpnStatus, { text: string; cls: string }> = {
  stopped: { text: "не подключён", cls: "dim" },
  starting: { text: "подключаемся…", cls: "" },
  running: { text: "защищён", cls: "" },
  stopping: { text: "отключаемся…", cls: "dim" },
  error: { text: "ошибка", cls: "warn" },
};

const MODE_LABEL: Record<VpnMode, string> = {
  proxy: "системный прокси",
  tun: "tun (весь трафик)",
};

// ── UTC clock ────────────────────────────────────────────────────────────────

function useUtcClock() {
  const [time, setTime] = useState("");
  useEffect(() => {
    const tick = () => {
      const d = new Date();
      const pad = (n: number) => String(n).padStart(2, "0");
      setTime(
        `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(
          d.getUTCSeconds()
        )} UTC`
      );
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, []);
  return time;
}

// ── SVG power icon ───────────────────────────────────────────────────────────

function PowerIcon() {
  return (
    <svg className="power-icon" viewBox="0 0 24 24">
      <path d="M12 4 V13" />
      <path d="M5.5 7.5 a9 9 0 1 0 13 0" />
    </svg>
  );
}

// ── Ping pill ────────────────────────────────────────────────────────────────

function pingClass(ms: number | null | undefined): string {
  if (ms == null) return "offline";
  if (ms < 80) return "fast";
  if (ms < 200) return "medium";
  return "slow";
}

function PingBadge({
  ms,
  loading,
}: {
  ms: number | null | undefined;
  loading: boolean;
}) {
  if (loading && ms === undefined) {
    return <span className="ping loading">…</span>;
  }
  if (ms == null) {
    return <span className="ping offline">— ms</span>;
  }
  return <span className={`ping ${pingClass(ms)}`}>{ms} ms</span>;
}

// ── Welcome card (первый запуск, нет подписок) ──────────────────────────────

function Welcome() {
  const subUrl = useSubscriptionStore((s) => s.url);
  const subLoading = useSubscriptionStore((s) => s.loading);
  const subError = useSubscriptionStore((s) => s.error);
  const setSubUrl = useSubscriptionStore((s) => s.setUrl);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);

  return (
    <div className="welcome">
      <div className="welcome-tag">— подключение за минуту</div>
      <h2 className="welcome-title">добавь подписку</h2>
      <p className="welcome-desc">
        вставь ссылку на свою подписку (URL вида&nbsp;
        <span className="bracket">https://sub.example.com/...</span>),
        приложение скачает список серверов и сразу замерит до них пинги.
      </p>
      <div className="row-input" style={{ marginTop: 8 }}>
        <input
          type="url"
          autoFocus
          value={subUrl}
          onChange={(e) => setSubUrl(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && fetchSubscription()}
          placeholder="https://sub.example.com/..."
          className="input"
        />
        <button
          type="button"
          disabled={subLoading || !subUrl.trim()}
          onClick={() => fetchSubscription()}
          className="btn-ghost"
        >
          {subLoading ? "…" : "загрузить"}
        </button>
      </div>
      {subError && <pre className="hero-error">{subError}</pre>}
      <div className="welcome-divider">
        <span>или</span>
      </div>
      <button
        type="button"
        onClick={openDashboard}
        className="btn-ghost"
        style={{ alignSelf: "stretch", padding: "12px" }}
      >
        войти в личный кабинет →
      </button>
      <p className="hint" style={{ marginTop: 4 }}>
        web.nemefisto.online · откроется в браузере
      </p>
    </div>
  );
}

// ── Toggle ───────────────────────────────────────────────────────────────────

function Toggle({
  on,
  onChange,
  disabled,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={disabled}
      onClick={() => !disabled && onChange(!on)}
      className={`toggle${on ? " is-on" : ""}${disabled ? " is-disabled" : ""}`}
    >
      <span className="toggle-knob" />
    </button>
  );
}

// ── Settings page ────────────────────────────────────────────────────────────

function SettingsPage({ onClose }: { onClose: () => void }) {
  const s = useSettingsStore();
  const subUrl = useSubscriptionStore((x) => x.url);
  const subHwid = useSubscriptionStore((x) => x.hwid);
  const deviceHwid = useSubscriptionStore((x) => x.deviceHwid);
  const setSubUrl = useSubscriptionStore((x) => x.setUrl);
  const setSubHwid = useSubscriptionStore((x) => x.setHwid);
  const fetchSubscription = useSubscriptionStore((x) => x.fetchSubscription);
  const subLoading = useSubscriptionStore((x) => x.loading);
  const subError = useSubscriptionStore((x) => x.error);
  const [hwidCopied, setHwidCopied] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const copyHwid = async () => {
    if (!deviceHwid) return;
    try {
      await navigator.clipboard.writeText(deviceHwid);
      setHwidCopied(true);
      setTimeout(() => setHwidCopied(false), 1500);
    } catch {
      // игнорируем
    }
  };

  return (
    <div className="settings-page">
      <div className="settings-frame">
        <header className="settings-header">
          <button
            type="button"
            onClick={onClose}
            className="back-btn"
            aria-label="назад"
          >
            ← назад
          </button>
          <h2 className="settings-title">настройки</h2>
        </header>

        <div className="settings-body">
        {/* ── Подписка ─────────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">подписка</div>
          <div className="row-input">
            <input
              type="url"
              value={subUrl}
              onChange={(e) => setSubUrl(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && fetchSubscription()}
              placeholder="https://sub.example.com/..."
              className="input"
            />
            <button
              type="button"
              disabled={subLoading || !subUrl.trim()}
              onClick={() => fetchSubscription()}
              className="btn-ghost"
            >
              {subLoading ? "…" : "обновить"}
            </button>
          </div>
          {subError && <pre className="hero-error">{subError}</pre>}
          <button
            type="button"
            onClick={openDashboard}
            className="btn-ghost"
            style={{ alignSelf: "flex-start", marginTop: 4 }}
          >
            личный кабинет →
          </button>
        </section>

        {/* ── При запуске ──────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">при запуске</div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">обновлять подписку</div>
              <div className="settings-row-hint">
                подгружать список серверов при каждом старте
              </div>
            </div>
            <Toggle
              on={s.refreshOnOpen}
              onChange={(v) => s.set("refreshOnOpen", v)}
            />
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">пинговать серверы</div>
              <div className="settings-row-hint">
                замерять задержку до всех серверов
              </div>
            </div>
            <Toggle
              on={s.pingOnOpen}
              onChange={(v) => s.set("pingOnOpen", v)}
            />
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">авто-подключение</div>
              <div className="settings-row-hint">
                подключаться к выбранному серверу при запуске
              </div>
            </div>
            <Toggle
              on={s.connectOnOpen}
              onChange={(v) => s.set("connectOnOpen", v)}
            />
          </div>
        </section>

        {/* ── Авто-обновление ──────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">авто-обновление</div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">обновлять подписку</div>
              <div className="settings-row-hint">
                в фоне через заданный интервал
              </div>
            </div>
            <Toggle
              on={s.autoRefresh}
              onChange={(v) => s.set("autoRefresh", v)}
            />
          </div>

          {s.autoRefresh && (
            <div className="settings-row">
              <div>
                <div className="settings-row-label">интервал (часы)</div>
              </div>
              <input
                type="number"
                min={1}
                max={48}
                value={s.autoRefreshHours}
                onChange={(e) =>
                  s.set(
                    "autoRefreshHours",
                    Math.max(1, Math.min(48, Number(e.target.value) || 1))
                  )
                }
                className="input input-num"
              />
            </div>
          )}
        </section>

        {/* ── Сортировка ──────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">сортировка серверов</div>
          {(
            [
              ["none", "без сортировки"],
              ["ping", "по пингу (от быстрых)"],
              ["name", "по алфавиту"],
            ] as [SortMode, string][]
          ).map(([value, label]) => (
            <label key={value} className="radio-row">
              <input
                type="radio"
                name="sort"
                checked={s.sort === value}
                onChange={() => s.set("sort", value)}
              />
              <span>{label}</span>
            </label>
          ))}
        </section>

        {/* ── Отправка данных ──────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">отправка данных</div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">передавать HWID</div>
              <div className="settings-row-hint">
                отправляется в заголовке x-hwid · сервер сам регистрирует
                устройство в подписке
              </div>
            </div>
            <Toggle
              on={s.sendHwid}
              onChange={(v) => s.set("sendHwid", v)}
            />
          </div>

          <div className="settings-row" style={{ flexDirection: "column", alignItems: "stretch", gap: 6 }}>
            <div className="settings-row-label">user agent</div>
            <input
              type="text"
              value={s.userAgent}
              onChange={(e) => s.set("userAgent", e.target.value)}
              placeholder={DEFAULT_USER_AGENT}
              className="input"
            />
            <div className="settings-row-hint">
              на UA `Happ/2.7.0` сервер отдаёт массив готовых Xray-конфигов
              с balancer-ом и burstObservatory. оставь пустым для дефолта.
            </div>
          </div>
        </section>

        {/* ── HWID информация ──────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">hwid устройства</div>
          <div className="hwid-row">
            <span className={"hwid-value" + (deviceHwid ? "" : " hwid-empty")}>
              {deviceHwid || "—"}
            </span>
            <button
              type="button"
              onClick={copyHwid}
              disabled={!deviceHwid}
              className="btn-ghost"
            >
              {hwidCopied ? "ок" : "копировать"}
            </button>
          </div>
          <p className="hint">
            machineguid windows · передаётся автоматически в каждом запросе
            подписки
          </p>

          <button
            type="button"
            onClick={() => setAdvancedOpen((v) => !v)}
            className="advanced-toggle"
          >
            {advancedOpen ? "▾ override hwid" : "▸ override hwid"}
          </button>
          {advancedOpen && (
            <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 8 }}>
              {subHwid.trim() && (
                <div className="warn-box">
                  <span className="warn-box-text">
                    активен override — приложение шлёт «{subHwid.slice(0, 12)}…» вместо системного hwid
                  </span>
                  <button
                    type="button"
                    onClick={() => setSubHwid("")}
                    className="btn-ghost"
                  >
                    сбросить
                  </button>
                </div>
              )}
              <input
                type="text"
                value={subHwid}
                onChange={(e) => setSubHwid(e.target.value)}
                placeholder={
                  deviceHwid || "оставь пустым чтобы использовать системный hwid"
                }
                className="input"
              />
            </div>
          )}
        </section>

        {/* ── Интерфейс ────────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">интерфейс</div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">язык</div>
              <div className="settings-row-hint">пока только русский</div>
            </div>
            <select className="select-field" disabled value="ru">
              <option value="ru">русский</option>
            </select>
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">тема</div>
              <div className="settings-row-hint">только тёмная</div>
            </div>
            <select className="select-field" disabled value="dark">
              <option value="dark">тёмная</option>
            </select>
          </div>
        </section>

        {/* ── Туннель ──────────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">туннель</div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">подключения из LAN</div>
              <div className="settings-row-hint">
                inbound слушает 0.0.0.0 — другие устройства в сети могут
                использовать этот прокси
              </div>
            </div>
            <Toggle
              on={s.allowLan}
              onChange={(v) => s.set("allowLan", v)}
            />
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">фрагментация TCP</div>
              <div className="settings-row-hint">этап 5 · скоро</div>
            </div>
            <Toggle on={false} onChange={() => {}} disabled />
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">мультиплексор (mux)</div>
              <div className="settings-row-hint">этап 5 · скоро</div>
            </div>
            <Toggle on={false} onChange={() => {}} disabled />
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">предпочитаемый IP</div>
              <div className="settings-row-hint">этап 5 · скоро</div>
            </div>
            <select className="select-field" disabled value="ipv4">
              <option value="ipv4">IPv4</option>
            </select>
          </div>
        </section>

        {/* ── Логи Xray ────────────────────────────────────────────────── */}
        <LogsBlock />

        {/* ── URL-схемы ────────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">url-схемы</div>
          <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5 }}>
            приложение реагирует на ссылки с префиксом <span className="bracket">nemefisto://</span>.
            бот может слать такие ссылки чтобы автоматически добавить подписку или
            переключить туннель.
          </p>
          <div className="schemes">
            <div className="scheme-row">
              <span className="scheme-url">nemefisto://add?url=&lt;url&gt;</span>
              <span className="scheme-desc">добавить подписку (URL должен быть encoded)</span>
            </div>
            <div className="scheme-row">
              <span className="scheme-url">nemefisto://connect</span>
              <span className="scheme-desc">подключить выбранный сервер</span>
            </div>
            <div className="scheme-row">
              <span className="scheme-url">nemefisto://disconnect</span>
              <span className="scheme-desc">остановить туннель</span>
            </div>
            <div className="scheme-row">
              <span className="scheme-url">nemefisto://toggle</span>
              <span className="scheme-desc">переключить состояние</span>
            </div>
          </div>
        </section>

        {/* ── О программе ──────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">о программе</div>
          <div className="about-grid">
            <span className="about-key">версия</span>
            <span className="about-val">v.{APP_VERSION} · build 2026.4</span>
            <span className="about-key">xray-core</span>
            <span className="about-val">26.x</span>
            <span className="about-key">сайт</span>
            <button
              type="button"
              onClick={openDashboard}
              className="about-link"
            >
              web.nemefisto.online
            </button>
            <span className="about-key">поддержка</span>
            <button
              type="button"
              onClick={openSupport}
              className="about-link"
            >
              @nemefistovpn_bot
            </button>
          </div>
        </section>

        {/* ── Сброс ────────────────────────────────────────────────────── */}
        <ResetBlock onAfterReset={onClose} />
        </div>
      </div>
    </div>
  );
}

// ── Logs viewer ──────────────────────────────────────────────────────────────

function LogsBlock() {
  const [text, setText] = useState("");
  const [loaded, setLoaded] = useState(false);

  const reload = async () => {
    try {
      const log = await invoke<string>("read_xray_log");
      setText(log || "(лог пустой)");
      setLoaded(true);
    } catch (e) {
      setText(String(e));
      setLoaded(true);
    }
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">логи xray</div>
      {!loaded ? (
        <button
          type="button"
          onClick={reload}
          className="btn-ghost"
          style={{ alignSelf: "flex-start" }}
        >
          показать логи
        </button>
      ) : (
        <>
          <pre className="logs-view">{text}</pre>
          <button
            type="button"
            onClick={reload}
            className="btn-ghost"
            style={{ alignSelf: "flex-start" }}
          >
            обновить
          </button>
        </>
      )}
    </section>
  );
}

// ── Reset block ──────────────────────────────────────────────────────────────

function ResetBlock({ onAfterReset }: { onAfterReset: () => void }) {
  const [confirm, setConfirm] = useState(false);
  const disconnect = useVpnStore((s) => s.disconnect);
  const settings = useSettingsStore();

  const doReset = async () => {
    try {
      await disconnect();
    } catch {
      // вне зависимости от результата чистим локальные данные
    }
    try {
      localStorage.clear();
    } catch {
      // приватный режим
    }
    settings.reset();
    onAfterReset();
    // перезагрузим страницу чтобы Zustand-stores переинициализировались
    window.location.reload();
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">сброс</div>
      {!confirm ? (
        <button
          type="button"
          onClick={() => setConfirm(true)}
          className="btn-danger"
          style={{ alignSelf: "flex-start" }}
        >
          сбросить приложение
        </button>
      ) : (
        <div className="warn-box" style={{ borderColor: "rgba(217,119,87,0.6)" }}>
          <span className="warn-box-text">
            это удалит подписку, hwid-override, все настройки и отключит туннель.
            продолжить?
          </span>
          <button
            type="button"
            onClick={() => setConfirm(false)}
            className="btn-ghost"
          >
            отмена
          </button>
          <button
            type="button"
            onClick={doReset}
            className="btn-danger"
          >
            да, сбросить
          </button>
        </div>
      )}
    </section>
  );
}

// ── Главный компонент ────────────────────────────────────────────────────────

function App() {
  const status = useVpnStore((s) => s.status);
  const errorMessage = useVpnStore((s) => s.errorMessage);
  const mode = useVpnStore((s) => s.mode);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const socksPort = useVpnStore((s) => s.socksPort);
  const httpPort = useVpnStore((s) => s.httpPort);
  const setMode = useVpnStore((s) => s.setMode);
  const selectServer = useVpnStore((s) => s.selectServer);
  const connect = useVpnStore((s) => s.connect);
  const disconnect = useVpnStore((s) => s.disconnect);
  const refresh = useVpnStore((s) => s.refresh);

  const servers = useSubscriptionStore((s) => s.servers);
  const pings = useSubscriptionStore((s) => s.pings);
  const pingsLoading = useSubscriptionStore((s) => s.pingsLoading);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  const loadCached = useSubscriptionStore((s) => s.loadCached);
  const loadDeviceHwid = useSubscriptionStore((s) => s.loadDeviceHwid);
  const pingAll = useSubscriptionStore((s) => s.pingAll);

  // UI состояние
  const [serverDrawer, setServerDrawer] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const utcTime = useUtcClock();

  // Settings
  const sort = useSettingsStore((x) => x.sort);
  const refreshOnOpen = useSettingsStore((x) => x.refreshOnOpen);
  const pingOnOpen = useSettingsStore((x) => x.pingOnOpen);
  const connectOnOpenSetting = useSettingsStore((x) => x.connectOnOpen);
  const autoRefresh = useSettingsStore((x) => x.autoRefresh);
  const autoRefreshHours = useSettingsStore((x) => x.autoRefreshHours);

  // ── Старт: refresh статуса VPN, кеш списка, HWID, on-open actions ─────────
  useEffect(() => {
    refresh();
    loadCached();
    loadDeviceHwid();
    if (refreshOnOpen) {
      void fetchSubscription();
    } else if (pingOnOpen) {
      // если не обновляем подписку, всё равно запускаем пинги по кешу
      void pingAll();
    }

    // Подписка на deep-link события (nemefisto://add | connect | ...)
    let unlisten: (() => void) | undefined;
    initDeepLinks().then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // только один раз на mount

  // ── Авто-подключение к последнему выбранному при старте ────────────────────
  const [didAutoConnect, setDidAutoConnect] = useState(false);
  useEffect(() => {
    if (didAutoConnect) return;
    if (!connectOnOpenSetting) return;
    if (selectedIndex === null || servers.length === 0) return;
    if (status !== "stopped") return;
    setDidAutoConnect(true);
    void connect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectOnOpenSetting, selectedIndex, servers.length, status]);

  // ── Auto-refresh подписки в фоне ──────────────────────────────────────────
  useEffect(() => {
    if (!autoRefresh) return;
    const ms = Math.max(1, autoRefreshHours) * 3600 * 1000;
    const id = window.setInterval(() => {
      void fetchSubscription();
    }, ms);
    return () => window.clearInterval(id);
  }, [autoRefresh, autoRefreshHours, fetchSubscription]);

  // ── Сортировка серверов ───────────────────────────────────────────────────
  const sortedIndices = useMemo(() => {
    const idx = servers.map((_, i) => i);
    if (sort === "none") return idx;
    if (sort === "name") {
      return idx.sort((a, b) =>
        servers[a].name.localeCompare(servers[b].name, "ru")
      );
    }
    if (sort === "ping") {
      return idx.sort((a, b) => {
        const pa = pings[a];
        const pb = pings[b];
        if (pa == null && pb == null) return 0;
        if (pa == null) return 1;
        if (pb == null) return -1;
        return pa - pb;
      });
    }
    return idx;
  }, [servers, sort, pings]);

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";
  const canConnect = selectedIndex !== null && !isBusy;
  const selectedServer =
    selectedIndex !== null ? servers[selectedIndex] : null;

  const onPowerClick = () => {
    if (isBusy) return;
    isRunning ? disconnect() : connect();
  };

  const pill = STATUS_PILL[status];
  const label = POWER_LABEL[status];

  return (
    <>
      <div className="grid-bg" />
      <div className="vignette" />
      <div className="scanlines" />

      <div className="app">
        <div className="frame">
          {/* ── Header ───────────────────────────────────────────────────── */}
          <header className="header">
            <div className="header-logo">
              <img src="/logo.png" alt="" />
              <span>nemefisto</span>
            </div>
            <div className="header-right">
              <div className="header-meta">
                <span className="blink">●</span>
                <span>{utcTime}</span>
              </div>
              <button
                type="button"
                className="icon-btn"
                onClick={openDashboard}
                aria-label="личный кабинет"
                title="личный кабинет (web.nemefisto.online)"
              >
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none"
                     stroke="currentColor" strokeWidth="1.6"
                     strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="12" cy="8" r="4" />
                  <path d="M4 21v-1a8 8 0 0 1 16 0v1" />
                </svg>
              </button>
              <button
                type="button"
                className="icon-btn"
                onClick={() => setSettingsOpen(true)}
                aria-label="настройки"
                title="настройки"
              >
                <svg viewBox="0 0 24 24" width="16" height="16" fill="none"
                     stroke="currentColor" strokeWidth="1.6"
                     strokeLinecap="round" strokeLinejoin="round">
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h0a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                </svg>
              </button>
            </div>
          </header>

          {/* ── Power stack ──────────────────────────────────────────────── */}
          <div className="power-stack">
            <div className={`status-pill ${pill.cls}`}>
              <span className="dot" />
              <span>{pill.label}</span>
            </div>

            <button
              type="button"
              className={`power-btn${isRunning ? " is-running" : ""}`}
              disabled={isRunning ? isBusy : !canConnect}
              onClick={onPowerClick}
              aria-label={isRunning ? "отключить" : "подключить"}
            >
              <PowerIcon />
              <span>
                {isBusy ? "…" : isRunning ? "отключить" : "подключить"}
              </span>
            </button>

            <div style={{ textAlign: "center" }}>
              <div className={`power-label ${label.cls}`}>{label.text}.</div>
              {isRunning && socksPort && (
                <div className="power-detail" style={{ marginTop: 6 }}>
                  socks5 127.0.0.1:{socksPort} · http :{httpPort}
                </div>
              )}
              {!isRunning && (
                <div className="power-detail" style={{ marginTop: 6 }}>
                  режим — {MODE_LABEL[mode]}
                </div>
              )}
            </div>
          </div>

          {/* ── Welcome (когда подписки ещё нет) ─────────────────────── */}
          {servers.length === 0 ? (
            <Welcome />
          ) : (
            /* ── Server pill (текущий выбранный) ─────────────────────── */
            <button
              type="button"
              className="server-pill"
              disabled={isRunning || isBusy}
              onClick={() => setServerDrawer((v) => !v)}
            >
              {selectedServer ? (
                <>
                  <span className="server-pill-num">
                    / {String(selectedIndex! + 1).padStart(2, "0")}
                  </span>
                  <span className="server-pill-name">
                    {selectedServer.name}
                  </span>
                  <PingBadge
                    ms={pings[selectedIndex!]}
                    loading={pingsLoading}
                  />
                  <span className="server-pill-arrow">
                    {serverDrawer ? "▾" : "▸"}
                  </span>
                </>
              ) : (
                <>
                  <span className="server-pill-empty">выбери сервер</span>
                  <span className="server-pill-arrow">▸</span>
                </>
              )}
            </button>
          )}

          {/* Server list — выпадает по клику на pill */}
          {serverDrawer && servers.length > 0 && (
            <div style={{ marginTop: 8 }}>
              <div className="server-list-head">
                <span>{servers.length} nodes</span>
                <button
                  type="button"
                  onClick={() => pingAll()}
                  disabled={pingsLoading}
                  className={`ping-refresh${pingsLoading ? " is-loading" : ""}`}
                  title="обновить пинги"
                  aria-label="обновить пинги"
                >
                  ↻
                </button>
              </div>
              <div className="server-list">
                {sortedIndices.map((i) => {
                  const s = servers[i];
                  return (
                    <div
                      key={i}
                      className={
                        "server-row" +
                        (selectedIndex === i ? " is-selected" : "") +
                        (isRunning ? " is-disabled" : "")
                      }
                      onClick={() => {
                        if (isRunning) return;
                        selectServer(i);
                        setServerDrawer(false);
                      }}
                    >
                      <span className="server-row-num">
                        {String(i + 1).padStart(2, "0")}
                      </span>
                      <span className="server-row-name">{s.name}</span>
                      <PingBadge ms={pings[i]} loading={pingsLoading} />
                      {selectedIndex === i && (
                        <span className="server-row-check">✓</span>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {errorMessage && (
            <pre className="hero-error" style={{ marginTop: 12 }}>
              {errorMessage}
            </pre>
          )}

          {/* Mode-segment (proxy/tun) — компактно перед footer */}
          <div className="mode-seg" style={{ marginTop: 12 }}>
            {(["proxy", "tun"] as VpnMode[]).map((m) => (
              <button
                key={m}
                type="button"
                disabled={isRunning || isBusy}
                onClick={() => setMode(m)}
                className={mode === m ? "is-active" : ""}
              >
                {m === "proxy" ? "системный прокси" : "tun"}
              </button>
            ))}
          </div>

          {/* ── Footer ──────────────────────────────────────────────────── */}
          <footer className="footer">
            <span>NO-LOGS · XRAY · VLESS · REALITY</span>
            <span>© 2026 NEMEFISTO · v.0.1.0</span>
          </footer>
        </div>
      </div>

      {/* Settings page — оверлей поверх основного UI */}
      {settingsOpen && (
        <SettingsPage onClose={() => setSettingsOpen(false)} />
      )}
    </>
  );
}

export default App;
