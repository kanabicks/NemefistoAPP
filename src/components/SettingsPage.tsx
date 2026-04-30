import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import {
  DEFAULT_USER_AGENT,
  PRESET_BACKGROUND,
  PRESET_BUTTON_STYLE,
  useSettingsStore,
  type Background,
  type ButtonStyle,
  type Preset,
  type SortMode,
  type Theme,
} from "../stores/settingsStore";
import { APP_VERSION } from "../lib/constants";
import { openDashboard, openSupport } from "../lib/openExternal";
import { useEffectiveSettings } from "../lib/hooks/useEffectiveSettings";
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
  const eff = useEffectiveSettings();
  const subUrl = useSubscriptionStore((x) => x.url);
  const subMeta = useSubscriptionStore((x) => x.meta);
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
          {subMeta?.title && (
            <div className="settings-row-hint" style={{ marginBottom: 8 }}>
              {subMeta.title} <span className="hint-badge">из подписки</span>
            </div>
          )}
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

          {/* 6.B — Autostart через Windows Task Scheduler. Состояние
              хранится в самой ОС (а не в localStorage), чтобы оно
              переживало переустановку и было видно пользователю в
              «Управление компьютером → Планировщик заданий». */}
          <AutostartRow />
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
                <div className="settings-row-label">
                  интервал (часы)
                  {!s.autoRefreshHoursTouched &&
                    subMeta?.updateIntervalHours != null && (
                      <span className="hint-badge" style={{ marginLeft: 8 }}>
                        из подписки
                      </span>
                    )}
                </div>
              </div>
              <input
                type="number"
                min={1}
                max={48}
                value={
                  !s.autoRefreshHoursTouched && subMeta?.updateIntervalHours
                    ? subMeta.updateIntervalHours
                    : s.autoRefreshHours
                }
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

          {/* Пресет — отдельная ось настройки. Уникальные палитры,
              недоступные через обычные «тема/фон/стиль». Когда выбран
              любой кроме «без пресета» — селекты ниже становятся
              недоступны (управляется пресетом). */}
          <div className="settings-row">
            <div>
              <div className="settings-row-label">
                пресет
                {eff.fromSubscription.preset && (
                  <span className="hint-badge" style={{ marginLeft: 8 }}>
                    из подписки
                  </span>
                )}
              </div>
              <div className="settings-row-hint">
                готовая уникальная стилизация
              </div>
            </div>
            <select
              className="select-field"
              value={eff.preset}
              onChange={(e) => s.set("preset", e.target.value as Preset)}
            >
              <option value="none">без пресета</option>
              <option value="fluent">fluent</option>
              <option value="cupertino">cupertino</option>
              <option value="vice">vice</option>
              <option value="arcade">arcade</option>
              <option value="glacier">glacier</option>
            </select>
          </div>

          {/* Если пресет активен, эти селекты — read-only с пометкой,
              что значение задаёт пресет. Это сохраняет UX-понятность:
              видно что и как переопределяется, а не «пропали настройки». */}
          {(() => {
            const presetActive = eff.preset !== "none";
            const effectiveBg = presetActive
              ? PRESET_BACKGROUND[eff.preset]
              : eff.background;
            const effectiveStyle = presetActive
              ? PRESET_BUTTON_STYLE[eff.preset]
              : eff.buttonStyle;
            const presetHint = "управляется пресетом";
            return (
              <>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      тема
                      {!presetActive && eff.fromSubscription.theme && (
                        <span className="hint-badge" style={{ marginLeft: 8 }}>
                          из подписки
                        </span>
                      )}
                    </div>
                    <div className="settings-row-hint">
                      {presetActive ? presetHint : "палитра приложения и кристалла"}
                    </div>
                  </div>
                  <select
                    className="select-field"
                    value={eff.theme}
                    disabled={presetActive}
                    onChange={(e) => s.set("theme", e.target.value as Theme)}
                  >
                    <option value="dark">тёмная</option>
                    <option value="light">светлая</option>
                    <option value="midnight">midnight</option>
                    <option value="sunset">sunset</option>
                    <option value="sand">sand</option>
                  </select>
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      фон
                      {!presetActive && eff.fromSubscription.background && (
                        <span className="hint-badge" style={{ marginLeft: 8 }}>
                          из подписки
                        </span>
                      )}
                    </div>
                    <div className="settings-row-hint">
                      {presetActive ? presetHint : "3d-сцена за интерфейсом"}
                    </div>
                  </div>
                  <select
                    className="select-field"
                    value={effectiveBg}
                    disabled={presetActive}
                    onChange={(e) => s.set("background", e.target.value as Background)}
                  >
                    <option value="crystal">кристалл</option>
                    <option value="tunnel">тоннель</option>
                    <option value="globe">глобус</option>
                    <option value="particles">частицы</option>
                  </select>
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      стиль кнопки
                      {!presetActive && eff.fromSubscription.buttonStyle && (
                        <span className="hint-badge" style={{ marginLeft: 8 }}>
                          из подписки
                        </span>
                      )}
                    </div>
                    <div className="settings-row-hint">
                      {presetActive ? presetHint : "оформление главной кнопки connect"}
                    </div>
                  </div>
                  <select
                    className="select-field"
                    value={effectiveStyle}
                    disabled={presetActive}
                    onChange={(e) => s.set("buttonStyle", e.target.value as ButtonStyle)}
                  >
                    <option value="glass">стекло</option>
                    <option value="flat">плоский</option>
                    <option value="neon">неон</option>
                    <option value="metallic">металл</option>
                  </select>
                </div>
              </>
            );
          })()}
        </section>

        {/* ── Anti-DPI (этап 10) ───────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">
            anti-dpi
            {!s.antiDpiTouched &&
              (subMeta?.fragmentationEnable != null ||
                subMeta?.noisesEnable != null ||
                subMeta?.serverResolveEnable != null) && (
                <span className="hint-badge" style={{ marginLeft: 8 }}>
                  из подписки
                </span>
              )}
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">фрагментация tcp</div>
              <div className="settings-row-hint">
                режет tls clienthello на куски — обходит большинство dpi
              </div>
            </div>
            <Toggle
              on={
                !s.antiDpiTouched && subMeta?.fragmentationEnable != null
                  ? subMeta.fragmentationEnable
                  : s.antiDpiFragmentation
              }
              onChange={(v) => s.set("antiDpiFragmentation", v)}
            />
          </div>

          {s.antiDpiFragmentation && (
            <>
              <div className="settings-row">
                <div>
                  <div className="settings-row-label">какие пакеты</div>
                </div>
                <select
                  className="select-field"
                  value={s.antiDpiFragmentationPackets}
                  onChange={(e) =>
                    s.set("antiDpiFragmentationPackets", e.target.value)
                  }
                >
                  <option value="tlshello">tlshello</option>
                  <option value="1-3">1-3</option>
                  <option value="all">все</option>
                </select>
              </div>
              <div className="settings-row">
                <div>
                  <div className="settings-row-label">длина (байт)</div>
                </div>
                <input
                  type="text"
                  className="input input-num"
                  value={s.antiDpiFragmentationLength}
                  onChange={(e) =>
                    s.set("antiDpiFragmentationLength", e.target.value)
                  }
                />
              </div>
              <div className="settings-row">
                <div>
                  <div className="settings-row-label">интервал (мс)</div>
                </div>
                <input
                  type="text"
                  className="input input-num"
                  value={s.antiDpiFragmentationInterval}
                  onChange={(e) =>
                    s.set("antiDpiFragmentationInterval", e.target.value)
                  }
                />
              </div>
            </>
          )}

          <div className="settings-row">
            <div>
              <div className="settings-row-label">шумовые пакеты</div>
              <div className="settings-row-hint">
                фейковые udp-пакеты для запутывания dpi
              </div>
            </div>
            <Toggle
              on={
                !s.antiDpiTouched && subMeta?.noisesEnable != null
                  ? subMeta.noisesEnable
                  : s.antiDpiNoises
              }
              onChange={(v) => s.set("antiDpiNoises", v)}
            />
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">doh-резолв сервера</div>
              <div className="settings-row-hint">
                адрес vpn-сервера резолвится через doh, минуя системный dns
                (помогает при dns-блокировках)
              </div>
            </div>
            <Toggle
              on={
                !s.antiDpiTouched && subMeta?.serverResolveEnable != null
                  ? subMeta.serverResolveEnable
                  : s.antiDpiServerResolve
              }
              onChange={(v) => s.set("antiDpiServerResolve", v)}
            />
          </div>

          {s.antiDpiServerResolve && (
            <>
              <div className="settings-row">
                <div>
                  <div className="settings-row-label">doh endpoint</div>
                </div>
                <input
                  type="url"
                  className="input"
                  value={s.antiDpiResolveDoH}
                  onChange={(e) => s.set("antiDpiResolveDoH", e.target.value)}
                />
              </div>
              <div className="settings-row">
                <div>
                  <div className="settings-row-label">bootstrap ip</div>
                </div>
                <input
                  type="text"
                  className="input input-num"
                  value={s.antiDpiResolveBootstrap}
                  onChange={(e) =>
                    s.set("antiDpiResolveBootstrap", e.target.value)
                  }
                />
              </div>
            </>
          )}
        </section>

        {/* ── Туннель ──────────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">туннель</div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">подключения из LAN</div>
              <div className="settings-row-hint">
                inbound слушает 0.0.0.0 — другие устройства в сети могут
                использовать этот прокси (логин/пароль показываются после connect)
              </div>
            </div>
            <Toggle
              on={s.allowLan}
              onChange={(v) => s.set("allowLan", v)}
            />
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">маскировка TUN-имени</div>
              <div className="settings-row-hint">
                имя адаптера выглядит как обычный сетевой интерфейс
                (wlan99 / Local Area Connection / Ethernet). помогает
                от приложений-шпионов которые детектят VPN по имени
                адаптера через GetAdaptersAddresses
              </div>
            </div>
            <Toggle
              on={s.tunMasking}
              onChange={(v) => s.set("tunMasking", v)}
            />
          </div>

          <div className="settings-row">
            <div>
              <div className="settings-row-label">kill switch</div>
              <div className="settings-row-hint">
                блокирует весь интернет если vpn упадёт — защита от
                утечек при reconnect/краше xray. ⚠️ если приложение
                крашнется, интернет останется заблокирован до ручной
                очистки firewall в admin-powershell
              </div>
            </div>
            <Toggle
              on={s.killSwitch}
              onChange={(v) => s.set("killSwitch", v)}
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

/** Toggle автозапуска (этап 6.B). Состояние читается прямо из Windows
 *  Task Scheduler, не из settings store, потому что user может удалить
 *  task через стандартный UI Windows и тогда настройка должна это
 *  отражать.*/
function AutostartRow() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        const ok = await invoke<boolean>("autostart_is_enabled");
        setEnabled(ok);
      } catch {
        setEnabled(false);
      }
    })();
  }, []);

  const toggle = async (v: boolean) => {
    setBusy(true);
    try {
      await invoke(v ? "autostart_enable" : "autostart_disable");
      setEnabled(v);
    } catch (e) {
      console.warn("[autostart] не удалось переключить:", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-row">
      <div>
        <div className="settings-row-label">запуск с системой</div>
        <div className="settings-row-hint">
          приложение само запустится при входе в windows (через task
          scheduler, без UAC)
        </div>
      </div>
      <Toggle
        on={enabled === true}
        onChange={toggle}
        disabled={busy || enabled === null}
      />
    </div>
  );
}
