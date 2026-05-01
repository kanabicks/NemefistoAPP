import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RoutingProfilesPanel } from "./RoutingProfilesPanel";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useRuntimeStore } from "../stores/runtimeStore";
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
import { APP_VERSION, GITHUB_URL, PRIVACY_URL, LICENSE_URL } from "../lib/constants";
import { openDashboard, openSupport } from "../lib/openExternal";
import { runLeakTest } from "../lib/leakTest";
import {
  exportBackupToDocuments,
  parseBackup,
  readBackupFile,
  useBackupModalStore,
} from "../lib/backup";
import { showToast } from "../stores/toastStore";
import { useEffectiveSettings } from "../lib/hooks/useEffectiveSettings";
import { Toggle } from "./Toggle";

/**
 * Полноэкранный оверлей настроек с двухуровневой навигацией.
 *
 * **Уровень 1** — список категорий (подписка / подключение / движок и т.д.).
 * **Уровень 2** — конкретная категория со всеми её настройками.
 *
 * Это сделано чтобы простыня из 16 секций не торчала вертикально на
 * 460px-окне. Состояние навигации локальное (`category`); при `null`
 * показываем categories-list, кнопка «← назад» в header возвращает на
 * уровень выше или закрывает Settings полностью.
 *
 * Все секции живут как fragment'ы внутри основного компонента,
 * чтобы не тащить ворох пропов в дочерние и сохранить хук-react state.
 */
type SettingsCategory =
  | "subscription"
  | "connection"
  | "engine"
  | "tunnel"
  | "security"
  | "routing"
  | "appearance"
  | "system";

type CategoryMeta = {
  id: SettingsCategory;
  icon: string;
  title: string;
  desc: string;
};

/** Метаданные категорий для рендера CategoryList. Иконки — эмодзи
 *  (без зависимости от иконочных шрифтов). Описание — короткая фраза
 *  что внутри, чтобы пользователь не открывал каждую наугад. */
const CATEGORIES: CategoryMeta[] = [
  {
    id: "subscription",
    icon: "📡",
    title: "Подписка",
    desc: "URL, обновление, user agent, HWID",
  },
  {
    id: "connection",
    icon: "🔌",
    title: "Подключение",
    desc: "поведение при запуске, сортировка серверов",
  },
  {
    id: "engine",
    icon: "⚙️",
    title: "Движок",
    desc: "Xray / Mihomo, правила приложений",
  },
  {
    id: "tunnel",
    icon: "🛡️",
    title: "Туннель",
    desc: "LAN, маскировка TUN-имени",
  },
  {
    id: "security",
    icon: "🔒",
    title: "Anti-DPI и защита",
    desc: "фрагментация, kill switch, leak protection",
  },
  {
    id: "routing",
    icon: "🗺️",
    title: "Маршрутизация",
    desc: "geosite/geoip профили, авто-группы",
  },
  {
    id: "appearance",
    icon: "🎨",
    title: "Интерфейс",
    desc: "пресет, тема, фон, плавающее окно",
  },
  {
    id: "system",
    icon: "🔧",
    title: "Система и о программе",
    desc: "автозапуск, обновления, история, логи, сброс",
  },
];

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

  // Активная категория. null = главный экран со списком категорий.
  const [category, setCategory] = useState<SettingsCategory | null>(null);

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

  // Header: разный заголовок и поведение «назад» в зависимости от уровня.
  const onBack = () => {
    if (category !== null) {
      setCategory(null);
    } else {
      onClose();
    }
  };
  const headerTitle =
    category === null
      ? "настройки"
      : CATEGORIES.find((c) => c.id === category)?.title.toLowerCase() ?? "настройки";

  return (
    <div className="settings-page">
      <div className="settings-frame">
        <header className="settings-header">
          <button
            type="button"
            onClick={onBack}
            className="back-btn"
            aria-label="назад"
          >
            ← назад
          </button>
          <h2 className="settings-title">{headerTitle}</h2>
        </header>

        <div className="settings-body">
          {category === null && (
            <CategoryList onSelect={setCategory} />
          )}

          {/* ── Подписка ─────────────────────────────────────────────────── */}
          {category === "subscription" && (
            <>
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
                {subMeta?.webPageUrl && (
                  <button
                    type="button"
                    onClick={openDashboard}
                    className="btn-ghost"
                    style={{ alignSelf: "flex-start", marginTop: 4 }}
                  >
                    личный кабинет →
                  </button>
                )}
              </section>

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

              <ComingSoonNote
                title="слияние нескольких подписок"
                desc="добавить 2-5 подписок параллельно, серверы из всех в одном списке с тегом источника"
              />
            </>
          )}

          {/* ── Подключение ─────────────────────────────────────────────── */}
          {category === "connection" && (
            <>
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

              <ComingSoonNote
                title="auto-failover"
                desc="при пинге выбранного сервера >3000мс автоматически переключаться на следующий по списку"
              />
              <ComingSoonNote
                title="доверенные wi-fi сети (SSID)"
                desc="список домашних wi-fi → vpn автоматически выключается. в гостевой сети — снова включается"
              />
              <ComingSoonNote
                title="глобальные горячие клавиши"
                desc="Ctrl+Shift+V toggle vpn, Ctrl+Shift+T переключить proxy/TUN"
              />
            </>
          )}

          {/* ── Движок ──────────────────────────────────────────────────── */}
          {category === "engine" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">vpn-ядро</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      движок
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
                      // 8.B: smart reconnect при смене движка — если активна
                      // VPN-сессия, гасим, рефетчим подписку с новым UA,
                      // поднимаем сессию обратно уже на новом движке.
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

              <AppRulesSection mihomoActive={mihomoActive} />

              <ComingSoonNote
                title="mihomo native TUN"
                desc="TUN-режим без tun2socks — Mihomo сам открывает WinTUN. правила приложений начнут работать в TUN тоже"
              />
            </>
          )}

          {/* ── Туннель ─────────────────────────────────────────────────── */}
          {category === "tunnel" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">сеть</div>
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
                    <div className="settings-row-label">только TUN-режим (strict)</div>
                    <div className="settings-row-hint">
                      скрыть переключатель proxy/tun на главном экране и
                      работать только через TUN-адаптер. без локального
                      SOCKS5-сокета на loopback — для параноиков, не желающих
                      оставлять никакой VPN-поверхности на 127.0.0.1
                    </div>
                  </div>
                  <Toggle
                    on={s.tunOnlyStrict}
                    onChange={(v) => s.set("tunOnlyStrict", v)}
                  />
                </div>
              </section>
            </>
          )}

          {/* ── Anti-DPI и защита ───────────────────────────────────────── */}
          {category === "security" && (
            <>
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

                {/* 8.B: фрагментация и шумы поддерживает только Xray. При
                    активном Mihomo — баннер «будут проигнорированы». */}
                {mihomoActive && (
                  <div className="hint-warning">
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

              <section className="settings-section">
                <div className="settings-section-title">kill switch</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">блокировать сеть при падении vpn</div>
                    <div className="settings-row-hint">
                      защита от утечек на уровне ядра windows (wfp). работает
                      даже если xray крашнется. защита от orphan-фильтров
                      тройная: dynamic-session + heartbeat-watchdog (60с) +
                      cleanup при старте helper-сервиса
                    </div>
                  </div>
                  <Toggle
                    on={s.killSwitch}
                    onChange={(v) => s.set("killSwitch", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">строгий режим</div>
                    <div className="settings-row-hint">
                      даже сам xray/mihomo может ходить только на vpn-сервер.
                      direct-маршруты (например <code>geosite:ru → DIRECT</code>{" "}
                      из вашего конфига) блокируются. для тех кто хочет
                      гарантированно «всё через vpn». ⚠️ ru-сайты в split-routing
                      перестанут открываться. требует включённого kill-switch
                    </div>
                  </div>
                  <Toggle
                    on={s.killSwitchStrict}
                    onChange={(v) => s.set("killSwitchStrict", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">блокировать прямые dns-запросы</div>
                    <div className="settings-row-hint">
                      все :53/udp+tcp кроме vpn-dns заблокированы. защищает
                      от dns-leak'а. ⚠️ в proxy-режиме может ломать приложения
                      которые используют системный dns мимо прокси (некоторые
                      игры, мессенджеры). лучше использовать в tun-режиме
                    </div>
                  </div>
                  <Toggle
                    on={s.dnsLeakProtection}
                    onChange={(v) => s.set("dnsLeakProtection", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">восстановить сеть</div>
                    <div className="settings-row-hint">
                      если интернет «полу-сломан» после краша / отключения —
                      одной кнопкой убирает wfp-фильтры, orphan tun-адаптеры,
                      half-default routes и системный прокси. безопасно жать
                      в любой момент когда vpn не активен
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn-ghost"
                    onClick={() => {
                      type RecoveryReport = {
                        kill_switch_cleaned: boolean;
                        orphan_resources_cleaned: boolean;
                        system_proxy_cleared: boolean;
                        errors: string[];
                      };
                      void invoke<RecoveryReport>("recover_network").then(
                        (r) => {
                          const cleaned = [
                            r.kill_switch_cleaned ? "wfp-фильтры" : null,
                            r.orphan_resources_cleaned
                              ? "tun + маршруты"
                              : null,
                            r.system_proxy_cleared ? "системный прокси" : null,
                          ].filter(Boolean);
                          if (r.errors.length === 0) {
                            showToast({
                              kind: "success",
                              title: "сеть восстановлена",
                              message:
                                cleaned.length > 0
                                  ? `очищено: ${cleaned.join(", ")}`
                                  : "ничего чистить не пришлось",
                            });
                          } else {
                            showToast({
                              kind: "warning",
                              title: "частично восстановлено",
                              message: `${
                                cleaned.length > 0
                                  ? `ок: ${cleaned.join(", ")}\n`
                                  : ""
                              }ошибки: ${r.errors.join("; ")}`,
                              durationMs: 12_000,
                            });
                          }
                        }
                      );
                    }}
                  >
                    восстановить
                  </button>
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">выгрузить диагностику</div>
                    <div className="settings-row-hint">
                      сохранит zip с логами xray, версией приложения, текущим
                      состоянием и списком запущенных vpn-процессов в папку
                      Documents. без телеметрии — только локально, ты сам
                      решаешь кому отправить файл если нужна помощь
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn-ghost"
                    onClick={() => {
                      void invoke<string>("export_diagnostics")
                        .then((path) => {
                          showToast({
                            kind: "success",
                            title: "диагностика сохранена",
                            message: `файл: ${path}\nоткроем папку?`,
                            durationMs: 8_000,
                          });
                          // Открываем explorer на родительской папке.
                          // Используем уже подключённый tauri-plugin-opener.
                          const dir = path.replace(/[\\/][^\\/]*$/, "");
                          void openUrl(dir).catch(() => {});
                        })
                        .catch((e) =>
                          showToast({
                            kind: "error",
                            title: "не получилось сохранить",
                            message: String(e),
                          })
                        );
                    }}
                  >
                    выгрузить
                  </button>
                </div>
              </section>

              <section className="settings-section">
                <div className="settings-section-title">проверка утечек</div>
                <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5, marginBottom: 8 }}>
                  делает два запроса параллельно: cloudflare cdn-trace для
                  публичного ip + страны (с fallback на ipwho.is для города)
                  и cloudflare doh whoami.cloudflare для ip dns-резолвера.
                  если ip-резолвера совпадает с твоим публичным — значит
                  dns-запросы видны как твои собственные (leak).
                </p>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">авто-проверка после подключения</div>
                    <div className="settings-row-hint">
                      через ~1.5 сек после connect показать тост с реальным ip
                    </div>
                  </div>
                  <Toggle
                    on={s.autoLeakTest}
                    onChange={(v) => s.set("autoLeakTest", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">проверить сейчас</div>
                    <div className="settings-row-hint">
                      запустить тест вручную — результат в правом нижнем углу
                    </div>
                  </div>
                  <button
                    type="button"
                    className="btn-ghost"
                    onClick={() => {
                      const v = useVpnStore.getState();
                      const port =
                        v.mode === "proxy" ? v.socksPort : null;
                      void runLeakTest(port);
                    }}
                  >
                    запустить
                  </button>
                </div>
              </section>

              <ComingSoonNote
                title="windows hello при запуске"
                desc="требовать аутентификацию (face/pin/fingerprint) при старте приложения. полезно для общих компьютеров"
              />
            </>
          )}

          {/* ── Маршрутизация ───────────────────────────────────────────── */}
          {category === "routing" && (
            <>
              <div className="settings-row-hint" style={{ marginBottom: 12 }}>
                routing-профили задают какие домены/IP идут через VPN, какие
                напрямую, какие блокируются. правила применяются к Xray и
                Mihomo при connect. для xray-json конфигов из подписки
                (с собственным routing) — НЕ применяются (приоритет у
                подписки)
              </div>
              <RoutingProfilesPanel />
              <section className="settings-section">
                <div className="settings-section-title">авто-шаблон</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      «минимальные правила РФ» если профиль не выбран
                    </div>
                    <div className="settings-row-hint">
                      когда нет активного routing-профиля — применяется
                      встроенный шаблон: <code>geosite:ru</code> +{" "}
                      <code>geoip:ru</code> + LAN → DIRECT, реклама
                      (<code>geosite:category-ads-all</code>) → BLOCK,
                      остальное → PROXY. полезно из коробки без импорта
                      внешних правил
                    </div>
                  </div>
                  <Toggle
                    on={s.autoApplyMinimalRuRules}
                    onChange={(v) => s.set("autoApplyMinimalRuRules", v)}
                  />
                </div>
              </section>
              <ComingSoonNote
                title="WFP per-app routing"
                desc="per-process правила в обоих движках через kernel-driver Windows Filtering Platform. альтернатива Mihomo PROCESS-NAME"
              />
            </>
          )}

          {/* ── Интерфейс ───────────────────────────────────────────────── */}
          {category === "appearance" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">пресет</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      готовый стиль
                      {eff.fromSubscription.preset && (
                        <span className="hint-badge" style={{ marginLeft: 8 }}>
                          из подписки
                        </span>
                      )}
                    </div>
                    <div className="settings-row-hint">
                      уникальная палитра, фон и стиль кнопки разом
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
              </section>

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
                  <section className="settings-section">
                    <div className="settings-section-title">тема и стиль</div>
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
                  </section>
                );
              })()}

              <section className="settings-section">
                <div className="settings-section-title">плавающее окно</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      мини-окно поверх всего
                    </div>
                    <div className="settings-row-hint">
                      статус vpn и текущая скорость ↑/↓ в маленьком
                      окошке. клик по точке — toggle, двойной клик по
                      окну — открыть главное
                    </div>
                  </div>
                  <Toggle
                    on={s.floatingWindow}
                    onChange={(v) => {
                      s.set("floatingWindow", v);
                      void invoke(
                        v ? "show_floating_window" : "hide_floating_window"
                      );
                    }}
                  />
                </div>
              </section>
            </>
          )}

          {/* ── Система и о программе ───────────────────────────────────── */}
          {category === "system" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">автозапуск</div>
                <AutostartRow />
              </section>

              <section className="settings-section">
                <div className="settings-section-title">горячие клавиши</div>
                <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5, marginBottom: 8 }}>
                  работают глобально, даже когда окно nemefisto скрыто. кликни поле, нажми
                  нужную комбинацию (минимум один модификатор: ctrl/alt/shift/win). esc — отмена,
                  backspace — очистить.
                </p>
                <ShortcutInput
                  label="включить / выключить vpn"
                  hint="как нажатие на главную кнопку — connect или disconnect"
                  value={s.shortcutToggleVpn}
                  onChange={(v) => s.set("shortcutToggleVpn", v)}
                />
                <ShortcutInput
                  label="показать / скрыть окно"
                  hint="как клик по иконке в системном трее"
                  value={s.shortcutShowHide}
                  onChange={(v) => s.set("shortcutShowHide", v)}
                />
                <ShortcutInput
                  label="переключить режим"
                  hint="proxy ↔ tun. срабатывает только когда vpn остановлен"
                  value={s.shortcutSwitchMode}
                  onChange={(v) => s.set("shortcutSwitchMode", v)}
                />
              </section>

              <section className="settings-section">
                <div className="settings-section-title">доверенные wi-fi</div>
                <TrustedWifiBlock />
              </section>

              <ComingSoonNote
                title="авто-обновление приложения"
                desc="клиент сам проверит наличие новой версии, скачает подписанный установщик и обновится — без захода на сайт"
              />
              <ComingSoonNote
                title="история сессий"
                desc="локальный лог connect/disconnect: время, сервер, режим, длительность, причина отключения"
              />
              <ComingSoonNote
                title="speed-test через VPN"
                desc="замер скорости через cloudflare CDN. опционально — авто-замер раз в неделю на всех серверах"
              />
              <BackupBlock />

              <LogsBlock />

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
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://export</span>
                    <span className="scheme-desc">выгрузить настройки в Documents</span>
                  </div>
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://import-from-url/&lt;url&gt;</span>
                    <span className="scheme-desc">скачать и предпросмотреть backup</span>
                  </div>
                </div>
              </section>

              <section className="settings-section">
                <div className="settings-section-title">о программе</div>
                <div className="about-grid">
                  <span className="about-key">версия</span>
                  <span className="about-val">v.{APP_VERSION} · build 2026.4</span>
                  <span className="about-key">xray-core</span>
                  <span className="about-val">26.x</span>
                  <span className="about-key">mihomo</span>
                  <span className="about-val">v1.19.24</span>
                  {subMeta?.webPageUrl && (
                    <>
                      <span className="about-key">личный кабинет</span>
                      <button
                        type="button"
                        onClick={openDashboard}
                        className="about-link"
                      >
                        {(() => {
                          try {
                            return new URL(subMeta.webPageUrl).host;
                          } catch {
                            return "ссылка";
                          }
                        })()}
                      </button>
                    </>
                  )}
                  <span className="about-key">поддержка</span>
                  <button
                    type="button"
                    onClick={openSupport}
                    className="about-link"
                  >
                    @nemefistovpn_bot
                  </button>
                  <span className="about-key">github</span>
                  <button
                    type="button"
                    onClick={() => void openUrl(GITHUB_URL)}
                    className="about-link"
                  >
                    kanabicks/NemefistoAPP
                  </button>
                  <span className="about-key">приватность</span>
                  <button
                    type="button"
                    onClick={() => void openUrl(PRIVACY_URL)}
                    className="about-link"
                  >
                    PRIVACY.md
                  </button>
                  <span className="about-key">лицензия</span>
                  <button
                    type="button"
                    onClick={() => void openUrl(LICENSE_URL)}
                    className="about-link"
                  >
                    MIT
                  </button>
                </div>
                <p
                  className="hint"
                  style={{
                    textTransform: "none",
                    letterSpacing: 0,
                    color: "var(--fg-dim)",
                    fontSize: 12,
                    lineHeight: 1.5,
                    marginTop: 12,
                  }}
                >
                  nemefisto не собирает телеметрию, не отправляет crash-репорты
                  «домой» и не имеет remote-control механизмов. все логи —
                  локально на этом компьютере. подробнее в PRIVACY.md.
                </p>
              </section>

              <ResetBlock onAfterReset={onClose} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// ─── Список категорий (главный экран Settings) ───────────────────────────────

function CategoryList({
  onSelect,
}: {
  onSelect: (c: SettingsCategory) => void;
}) {
  return (
    <div className="settings-categories">
      {CATEGORIES.map((c) => (
        <button
          key={c.id}
          type="button"
          className="settings-category"
          onClick={() => onSelect(c.id)}
        >
          <span className="settings-category-icon" aria-hidden>
            {c.icon}
          </span>
          <span className="settings-category-text">
            <span className="settings-category-title">{c.title}</span>
            <span className="settings-category-desc">{c.desc}</span>
          </span>
          <span className="settings-category-arrow" aria-hidden>
            ›
          </span>
        </button>
      ))}
    </div>
  );
}

// ─── Плашка «скоро» для будущих фич ─────────────────────────────────────────

function ComingSoonNote({ title, desc }: { title: string; desc: string }) {
  return (
    <section className="settings-section coming-soon">
      <div className="coming-soon-row">
        <span className="coming-soon-badge">скоро</span>
        <div className="coming-soon-text">
          <div className="coming-soon-title">{title}</div>
          <div className="coming-soon-desc">{desc}</div>
        </div>
      </div>
    </section>
  );
}

// ── App rules (per-process routing, 8.D) ─────────────────────────────────────

/**
 * Секция Settings → Движок → «правила приложений (Mihomo)». Список
 * правил `<exe-name> → PROXY|DIRECT|BLOCK` + форма добавления нового.
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
        <div className="hint-warning">
          активен xray — правила сейчас не применяются (на windows
          per-process routing работает только в mihomo через
          PROCESS-NAME matcher). переключи движок чтобы заработало
        </div>
      )}

      {mihomoActive && tunMode && (
        <div className="hint-danger">
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

// ── Backup block (12.D) ────────────────────────────────────────────────────

/**
 * 12.D — экспорт/импорт настроек.
 *
 * - **выгрузить в файл** → пишем JSON в `~/Documents/nemefisto-backup-<ts>.json`,
 *   показываем toast с путём.
 * - **загрузить из файла** → `<input type="file">` + FileReader →
 *   `parseBackup` → `useBackupModalStore.show(...)` → preview-модалка
 *   с diff'ом и кнопкой «применить».
 *
 * Также активны deep-link'и `nemefisto://export` и
 * `nemefisto://import-from-url/<url>` (см. lib/deepLinks.ts).
 */
function BackupBlock() {
  const [busy, setBusy] = useState(false);

  const onExport = async () => {
    setBusy(true);
    try {
      const path = await exportBackupToDocuments();
      showToast({
        kind: "success",
        title: "выгружено",
        message: path,
        durationMs: 8000,
      });
    } catch (e) {
      showToast({ kind: "error", title: "не удалось выгрузить", message: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const onImport = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = ""; // позволяет выбрать тот же файл повторно
    if (!file) return;
    void readBackupFile(file)
      .then(parseBackup)
      .then((backup) => {
        useBackupModalStore.getState().show(backup);
      })
      .catch((err) => {
        showToast({
          kind: "error",
          title: "не удалось прочитать backup",
          message: String(err),
        });
      });
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">backup настроек</div>
      <p
        className="hint"
        style={{
          textTransform: "none",
          letterSpacing: 0,
          color: "var(--fg-dim)",
          fontSize: 12,
          lineHeight: 1.5,
          marginBottom: 8,
        }}
      >
        выгружает все настройки + URL подписки в JSON-файл (HWID и
        прочие машинно-зависимые данные не попадают). при импорте сначала
        покажется превью изменений.
      </p>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <button
          type="button"
          onClick={onExport}
          disabled={busy}
          className="btn-ghost"
        >
          выгрузить в файл
        </button>
        <label className="btn-ghost" style={{ cursor: "pointer" }}>
          загрузить из файла
          <input
            type="file"
            accept="application/json,.json"
            style={{ display: "none" }}
            onChange={onImport}
          />
        </label>
      </div>
    </section>
  );
}

// ── Logs block ────────────────────────────────────────────────────────────

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

// ─── Запись комбинации горячих клавиш (этап 13.N) ────────────────────────────

/**
 * Поле для записи accelerator'а вида `Ctrl+Shift+V`. Клик — переходит
 * в режим записи (фокус), любая комбинация с модификатором → сохранение.
 *
 * - **Esc** — отмена записи без сохранения.
 * - **Backspace / Delete** — очищает (`null` → клавиша снимается).
 * - Только клавиши с хотя бы одним модификатором (`Ctrl/Alt/Shift/Win`)
 *   принимаются — иначе любая буква сохранилась бы как hotkey, что
 *   ломает обычный набор текста в других приложениях.
 */
function ShortcutInput({
  value,
  onChange,
  label,
  hint,
}: {
  value: string | null;
  onChange: (v: string | null) => void;
  label: string;
  hint?: string;
}) {
  const [recording, setRecording] = useState(false);

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape") {
      setRecording(false);
      return;
    }
    if (e.key === "Backspace" || e.key === "Delete") {
      onChange(null);
      setRecording(false);
      return;
    }

    // Сами модификаторы как «нажатия» — игнор (ждём «настоящую» клавишу).
    if (
      e.key === "Control" ||
      e.key === "Shift" ||
      e.key === "Alt" ||
      e.key === "Meta" ||
      e.key === "OS"
    ) {
      return;
    }

    // Минимум один модификатор — иначе hotkey пересекается с обычным вводом.
    const hasMod = e.ctrlKey || e.altKey || e.shiftKey || e.metaKey;
    if (!hasMod) return;

    // Маппим event.code в accelerator-key (не зависит от раскладки).
    let key: string | null = null;
    const code = e.code;
    if (code.startsWith("Key") && code.length === 4) {
      key = code.slice(3); // KeyV → V
    } else if (code.startsWith("Digit") && code.length === 6) {
      key = code.slice(5); // Digit1 → 1
    } else if (/^F([1-9]|1\d|2[0-4])$/.test(code)) {
      key = code; // F1..F24
    } else if (code === "Space") {
      key = "Space";
    } else if (code === "Enter") {
      key = "Enter";
    } else if (code === "Tab") {
      key = "Tab";
    } else if (
      code === "ArrowUp" ||
      code === "ArrowDown" ||
      code === "ArrowLeft" ||
      code === "ArrowRight"
    ) {
      key = code.replace("Arrow", "");
    } else if (code === "Home" || code === "End" || code === "PageUp" || code === "PageDown") {
      key = code;
    } else if (code === "Insert") {
      key = "Insert";
    } else {
      return; // неподдерживаемый клавиатурный код
    }

    const parts: string[] = [];
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.altKey) parts.push("Alt");
    if (e.shiftKey) parts.push("Shift");
    if (e.metaKey) parts.push("Super");
    parts.push(key);

    onChange(parts.join("+"));
    setRecording(false);
  };

  return (
    <div className="settings-row shortcut-row">
      <div>
        <div className="settings-row-label">{label}</div>
        {hint && <div className="settings-row-hint">{hint}</div>}
      </div>
      <div
        className={`shortcut-input${recording ? " is-recording" : ""}`}
        tabIndex={0}
        role="button"
        onClick={() => setRecording(true)}
        onBlur={() => setRecording(false)}
        onKeyDown={recording ? onKeyDown : undefined}
      >
        {recording
          ? "нажми комбинацию…"
          : value ?? "не задано"}
        {!recording && value && (
          <button
            type="button"
            className="shortcut-clear"
            onClick={(e) => {
              e.stopPropagation();
              onChange(null);
            }}
            title="очистить"
          >
            ×
          </button>
        )}
      </div>
    </div>
  );
}

// ─── Доверенные Wi-Fi сети (этап 13.M) ───────────────────────────────────────

/**
 * Список SSID + действие при подключении к ним. Сверху — текущий
 * SSID с кнопкой «добавить эту сеть» (если есть Wi-Fi подключение
 * и сеть ещё не в списке). Ниже — список с кнопками удаления и
 * input для ручного ввода (если адаптера Wi-Fi нет, например на
 * стационарном ПК).
 */
function TrustedWifiBlock() {
  const trustedSsids = useSettingsStore((s) => s.trustedSsids);
  const trustedSsidAction = useSettingsStore((s) => s.trustedSsidAction);
  const autoConnectOnLeave = useSettingsStore((s) => s.autoConnectOnLeave);
  const setOpt = useSettingsStore((s) => s.set);
  const currentSsid = useRuntimeStore((s) => s.currentSsid);

  const [manualInput, setManualInput] = useState("");

  const isCurrentInList =
    currentSsid !== null && trustedSsids.includes(currentSsid);

  const addSsid = (name: string) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    if (trustedSsids.includes(trimmed)) return;
    setOpt("trustedSsids", [...trustedSsids, trimmed]);
  };
  const removeSsid = (name: string) => {
    setOpt(
      "trustedSsids",
      trustedSsids.filter((s) => s !== name)
    );
  };

  return (
    <>
      <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5, marginBottom: 8 }}>
        в доверенной сети vpn автоматически отключается. при возврате
        в обычную — может включиться обратно (если включена опция ниже).
        работает только с wi-fi (по ssid из netsh), ethernet/мобильный
        интернет считаются обычной сетью.
      </p>

      <div className="trusted-current">
        <span className="trusted-current-label">текущая сеть:</span>
        <span className="trusted-current-name">
          {currentSsid ? currentSsid : "—"}
        </span>
        {currentSsid && !isCurrentInList && (
          <button
            type="button"
            className="btn-ghost"
            onClick={() => addSsid(currentSsid)}
            style={{ fontSize: 12, padding: "4px 10px" }}
          >
            + добавить
          </button>
        )}
        {isCurrentInList && (
          <span className="trusted-current-badge">в списке</span>
        )}
      </div>

      {trustedSsids.length > 0 && (
        <div className="app-rules-list" style={{ marginTop: 10 }}>
          {trustedSsids.map((ssid) => (
            <div key={ssid} className="app-rule-row">
              <span className="app-rule-exe">{ssid}</span>
              {ssid === currentSsid && (
                <span className="trusted-current-badge">текущая</span>
              )}
              <button
                type="button"
                className="app-rule-del"
                onClick={() => removeSsid(ssid)}
                title="удалить"
                style={{ marginLeft: "auto" }}
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="app-rule-add" style={{ marginTop: 10, gridTemplateColumns: "1fr auto" }}>
        <input
          type="text"
          className="input"
          placeholder="ввести имя сети вручную"
          value={manualInput}
          onChange={(e) => setManualInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              addSsid(manualInput);
              setManualInput("");
            }
          }}
        />
        <button
          type="button"
          className="btn-ghost"
          onClick={() => {
            addSsid(manualInput);
            setManualInput("");
          }}
          disabled={!manualInput.trim()}
        >
          добавить
        </button>
      </div>

      <div className="settings-row" style={{ marginTop: 12 }}>
        <div>
          <div className="settings-row-label">при подключении к доверенной</div>
          <div className="settings-row-hint">
            что делать когда мы попали в одну из сетей выше
          </div>
        </div>
        <select
          className="select-field"
          value={trustedSsidAction}
          onChange={(e) =>
            setOpt(
              "trustedSsidAction",
              e.target.value as "ignore" | "disconnect"
            )
          }
        >
          <option value="ignore">ничего</option>
          <option value="disconnect">отключить vpn</option>
        </select>
      </div>

      <div className="settings-row">
        <div>
          <div className="settings-row-label">авто-включение при выходе</div>
          <div className="settings-row-hint">
            когда уходим из доверенной сети — переподключиться, если
            vpn отключали именно мы по этому правилу
          </div>
        </div>
        <Toggle
          on={autoConnectOnLeave}
          onChange={(v) => setOpt("autoConnectOnLeave", v)}
          disabled={trustedSsidAction === "ignore"}
        />
      </div>
    </>
  );
}
