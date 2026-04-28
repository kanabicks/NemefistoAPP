import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import {
  DEFAULT_USER_AGENT,
  useSettingsStore,
  type SortMode,
  type Theme,
} from "../stores/settingsStore";
import { APP_VERSION } from "../lib/constants";
import { openDashboard, openSupport } from "../lib/openExternal";
import { Toggle } from "./Toggle";

/**
 * Полноэкранный оверлей настроек.
 *
 * Внутри сгруппированы секции: подписка, поведение при запуске,
 * авто-обновление, сортировка, отправка данных, HWID, интерфейс,
 * параметры туннеля, логи Xray, URL-схемы, инфо, сброс.
 *
 * LogsBlock и ResetBlock держим тут же — они нигде вне settings
 * не используются.
 */
export function SettingsPage({ onClose }: { onClose: () => void }) {
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
              <div className="settings-row-label">тема</div>
              <div className="settings-row-hint">
                светлая или тёмная палитра приложения
              </div>
            </div>
            <select
              className="select-field"
              value={s.theme}
              onChange={(e) => s.set("theme", e.target.value as Theme)}
            >
              <option value="dark">тёмная</option>
              <option value="light">светлая</option>
              <option value="midnight">midnight</option>
              <option value="sunset">sunset</option>
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
