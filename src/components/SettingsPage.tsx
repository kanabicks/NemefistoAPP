import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import {
  DEFAULT_USER_AGENT_MIHOMO,
  DEFAULT_USER_AGENT_XRAY,
  PRESET_BACKGROUND,
  PRESET_BUTTON_STYLE,
  useSettingsStore,
  type AppRule,
  type AppRuleAction,
  type Background,
  type ButtonStyle,
  type Engine,
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
  // 8.B: для smart-reconnect при смене движка нужны connect/disconnect
  // и текущий статус — иначе пользователь меняет engine, подписка
  // refetch'ится, но активная сессия остаётся на старом движке.
  const vpnStatus = useVpnStore((s) => s.status);
  const vpnConnect = useVpnStore((s) => s.connect);
  const vpnDisconnect = useVpnStore((s) => s.disconnect);
  const [hwidCopied, setHwidCopied] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);

  // 8.B: эффективный движок (с override-логикой для server-driven UX).
  // Используется в anti-DPI секции, чтобы скрыть/предупредить о
  // фрагментации/шумах, которые работают только в Xray.
  const effectiveEngine: Engine =
    !s.engineTouched && (subMeta?.engine === "mihomo" || subMeta?.engine === "xray")
      ? (subMeta.engine as Engine)
      : s.engine;
  const mihomoActive = effectiveEngine === "mihomo";

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
            <div className="settings-row-label">
              user agent
              {!s.userAgentTouched && (
                <span className="hint-badge" style={{ marginLeft: 8 }}>
                  авто по движку
                </span>
              )}
            </div>
            <input
              type="text"
              value={s.userAgent}
              onChange={(e) => s.set("userAgent", e.target.value)}
              placeholder={mihomoActive ? DEFAULT_USER_AGENT_MIHOMO : DEFAULT_USER_AGENT_XRAY}
              className="input"
            />
            <div className="settings-row-hint">
              автоматически: <b>Happ/2.7.0</b> для Xray (Marzban-style xray-json
              с готовым routing), <b>clash-verge</b> для Mihomo (clash YAML).
              если правишь вручную — фиксируется как есть, на оба движка.
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

          {/* 8.B: фрагментация и шумы поддерживает только Xray (через
              freedom-fragment outbound). При активном Mihomo показываем
              подсказку — toggle'ы остаются (UI-консистентность), но при
              connect Mihomo их просто игнорирует. doh-резолв работает
              в обоих движках (через dns.nameserver/hosts). */}
          {mihomoActive && (
            <div
              className="settings-row-hint"
              style={{
                marginBottom: 8,
                padding: "6px 10px",
                background: "rgba(255,183,77,0.06)",
                borderRadius: 4,
                borderLeft: "2px solid #ffb74d",
              }}
            >
              активен mihomo — фрагментация и шумы будут проигнорированы
              (поддерживаются только xray). doh-резолв продолжает работать
              через dns mihomo
            </div>
          )}

          <div className="settings-row">
            <div>
              <div className="settings-row-label">фрагментация tcp</div>
              <div className="settings-row-hint">
                режет tls clienthello на куски — обходит большинство dpi
                {mihomoActive && " (только xray)"}
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

        {/* ── Движок (8.B) ─────────────────────────────────────────────── */}
        <section className="settings-section">
          <div className="settings-section-title">движок</div>
          <div className="settings-row">
            <div>
              <div className="settings-row-label">
                vpn-ядро
                {!s.engineTouched && subMeta?.engine && (
                  <span className="hint-badge" style={{ marginLeft: 8 }}>
                    из подписки
                  </span>
                )}
              </div>
              <div className="settings-row-hint">
                <b>xray</b> — REALITY/Vision/XHTTP, низкая латентность, оптимально для
                vless/vmess/trojan/ss/hy2/wireguard. <b>mihomo</b> — нужен для
                tuic / anytls / mieru и для per-process routing
              </div>
            </div>
            <select
              className="select-field"
              value={
                !s.engineTouched && subMeta?.engine === "mihomo"
                  ? "mihomo"
                  : !s.engineTouched && subMeta?.engine === "xray"
                  ? "xray"
                  : s.engine
              }
              onChange={(e) => {
                const next = e.target.value as Engine;
                s.set("engine", next);
                if (!subUrl.trim()) return;
                // 8.B: при смене движка нужно три действия атомарно —
                // (1) если активна VPN-сессия, погасить её (она бежит
                //     на старом движке);
                // (2) перезапросить подписку с новым UA, чтобы получить
                //     конфиг под нужный формат (xray-json vs clash YAML);
                // (3) если до смены был connected — поднять сессию
                //     обратно уже на новом движке.
                // Без шага 3 пользователь вынужден сам тыкать connect
                // после смены — UX плохой. Без шага 1 disconnect
                // происходил бы при следующем connect, но трафик в
                // паузе уходит мимо VPN.
                const wasRunning = vpnStatus === "running";
                void (async () => {
                  if (wasRunning) await vpnDisconnect();
                  await fetchSubscription();
                  if (wasRunning) await vpnConnect();
                })();
              }}
            >
              <option value="xray">Xray</option>
              <option value="mihomo">Mihomo</option>
            </select>
          </div>
        </section>

        {/* ── Правила приложений (8.D) ─────────────────────────────────── */}
        <AppRulesSection mihomoActive={mihomoActive} />


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

// ── App rules (per-process routing, 8.D) ─────────────────────────────────────

/**
 * Секция Settings → «правила приложений (Mihomo)». Список правил
 * `<exe-name> → PROXY|DIRECT|BLOCK` + форма добавления нового.
 *
 * Mihomo нативно умеет PROCESS-NAME matcher; Xray на Windows — нет
 * (планируется через WFP в 13.G). Если активен Xray — баннер сверху
 * предупреждает что правила игнорируются. Хранятся всегда — при
 * переключении движка на Mihomo сразу применятся.
 */
function AppRulesSection({ mihomoActive }: { mihomoActive: boolean }) {
  const rules = useSettingsStore((s) => s.appRules);
  const set = useSettingsStore((s) => s.set);
  // 8.D: PROCESS-NAME matcher Mihomo на Windows работает только когда
  // соединение приходит к Mihomo напрямую от приложения (proxy-режим).
  // В TUN-режиме между приложением и Mihomo стоит tun2socks — Mihomo
  // видит PID tun2socks, а не исходного приложения, и matcher не
  // срабатывает. Это уйдёт когда сделаем 13.L (Mihomo built-in TUN
  // через gVisor — там Mihomo сам видит ядерный PID).
  const vpnMode = useVpnStore((s) => s.mode);
  const tunMode = vpnMode === "tun";

  const [draftExe, setDraftExe] = useState("");
  const [draftAction, setDraftAction] = useState<AppRuleAction>("direct");
  const [draftComment, setDraftComment] = useState("");

  const addRule = () => {
    const exe = draftExe.trim().toLowerCase();
    if (!exe) return;
    // Дедупликация по exe — одна запись на исполняемый файл, при
    // повторном добавлении обновляется action/comment.
    const filtered = rules.filter((r) => r.exe.toLowerCase() !== exe);
    const next: AppRule[] = [
      ...filtered,
      {
        exe,
        action: draftAction,
        comment: draftComment.trim() || undefined,
      },
    ];
    set("appRules", next);
    setDraftExe("");
    setDraftComment("");
  };

  const removeRule = (exe: string) => {
    set(
      "appRules",
      rules.filter((r) => r.exe !== exe)
    );
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">правила приложений</div>

      {!mihomoActive && (
        <div
          className="settings-row-hint"
          style={{
            marginBottom: 8,
            padding: "6px 10px",
            background: "rgba(255,183,77,0.06)",
            borderRadius: 4,
            borderLeft: "2px solid #ffb74d",
          }}
        >
          активен xray — правила сейчас не применяются (на windows
          per-process routing работает только в mihomo через
          PROCESS-NAME matcher). переключи движок чтобы заработало
        </div>
      )}

      {mihomoActive && tunMode && (
        <div
          className="settings-row-hint"
          style={{
            marginBottom: 8,
            padding: "6px 10px",
            background: "rgba(229,115,115,0.07)",
            borderRadius: 4,
            borderLeft: "2px solid #e57373",
          }}
        >
          <b>в TUN-режиме правила сейчас не работают.</b> между приложением
          и Mihomo стоит tun2socks, и Mihomo видит соединения от него,
          а не от исходного процесса. переключи режим на <b>proxy</b>
          чтобы правила применились. полное решение для TUN придёт с
          этапом 13.L (Mihomo native TUN)
        </div>
      )}

      <div className="settings-row-hint" style={{ marginBottom: 10 }}>
        правила вида <b>«&lt;exe&gt; → action»</b> применяются Mihomo
        к процессам по имени исполняемого файла. например, можно
        пустить telegram через VPN, а steam — направить direct.
        имя exe берётся из диспетчера задач (телеграм.exe, steam.exe).
        работают только в <b>proxy-режиме</b>
      </div>

      {rules.length > 0 && (
        <div className="app-rules-list">
          {rules.map((r) => (
            <div key={r.exe} className="app-rule-row">
              <span className="app-rule-exe">{r.exe}</span>
              <span
                className={`app-rule-badge action-${r.action}`}
                title={
                  r.action === "proxy"
                    ? "через VPN"
                    : r.action === "direct"
                    ? "напрямую, мимо VPN"
                    : "блокируется"
                }
              >
                {r.action}
              </span>
              {r.comment && (
                <span className="app-rule-comment">{r.comment}</span>
              )}
              <button
                type="button"
                className="app-rule-del"
                onClick={() => removeRule(r.exe)}
                title="удалить правило"
                aria-label="удалить"
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="app-rule-add">
        <input
          type="text"
          className="input"
          value={draftExe}
          onChange={(e) => setDraftExe(e.target.value)}
          placeholder="telegram.exe"
          onKeyDown={(e) => e.key === "Enter" && addRule()}
        />
        <select
          className="select-field"
          value={draftAction}
          onChange={(e) => setDraftAction(e.target.value as AppRuleAction)}
        >
          <option value="direct">direct</option>
          <option value="proxy">proxy</option>
          <option value="block">block</option>
        </select>
        <input
          type="text"
          className="input"
          value={draftComment}
          onChange={(e) => setDraftComment(e.target.value)}
          placeholder="заметка (опционально)"
          onKeyDown={(e) => e.key === "Enter" && addRule()}
        />
        <button
          type="button"
          className="btn-ghost"
          onClick={addRule}
          disabled={!draftExe.trim()}
        >
          добавить
        </button>
      </div>
    </section>
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

/**
 * Блок «сброс» (этап 12.A). Две раздельные кнопки:
 * - **сбросить настройки** — только `settingsStore.reset()`, подписка
 *   и HWID-override остаются. Полезно когда подкрутил тему/anti-DPI
 *   до сломанного состояния, а перенастраивать подписку не хочется.
 * - **удалить всё** — settings + подписка + HWID + dismissed-set
 *   объявлений. Это полный wipe localStorage.
 *
 * Двойной confirm-step для каждой — чтобы случайный клик не уничтожил
 * данные. Active-confirm подсвечивает только одну из двух — пользователь
 * понимает что именно собирается сделать.
 */
function ResetBlock({ onAfterReset }: { onAfterReset: () => void }) {
  type Pending = null | "settings" | "all";
  const [pending, setPending] = useState<Pending>(null);
  const disconnect = useVpnStore((s) => s.disconnect);
  const settings = useSettingsStore();

  const doResetSettings = () => {
    settings.reset();
    setPending(null);
    onAfterReset();
  };

  const doResetAll = async () => {
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

      {pending === null && (
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <button
            type="button"
            onClick={() => setPending("settings")}
            className="btn-ghost"
          >
            сбросить настройки
          </button>
          <button
            type="button"
            onClick={() => setPending("all")}
            className="btn-danger"
          >
            удалить всё
          </button>
        </div>
      )}

      {pending === "settings" && (
        <div className="warn-box" style={{ borderColor: "rgba(217,119,87,0.4)" }}>
          <span className="warn-box-text">
            настройки вернутся к дефолтным (тема, anti-DPI, движок,
            правила приложений). <b>подписка и hwid останутся.</b>
            продолжить?
          </span>
          <button
            type="button"
            onClick={() => setPending(null)}
            className="btn-ghost"
          >
            отмена
          </button>
          <button
            type="button"
            onClick={doResetSettings}
            className="btn-danger"
          >
            да, сбросить
          </button>
        </div>
      )}

      {pending === "all" && (
        <div className="warn-box" style={{ borderColor: "rgba(217,119,87,0.6)" }}>
          <span className="warn-box-text">
            <b>это удалит абсолютно всё:</b> подписку, hwid-override,
            все настройки, dismissed-объявления. отключит туннель и
            перезагрузит приложение
          </span>
          <button
            type="button"
            onClick={() => setPending(null)}
            className="btn-ghost"
          >
            отмена
          </button>
          <button
            type="button"
            onClick={doResetAll}
            className="btn-danger"
          >
            да, удалить всё
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
