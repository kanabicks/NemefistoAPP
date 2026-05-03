import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RoutingProfilesPanel } from "./RoutingProfilesPanel";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useRuntimeStore } from "../stores/runtimeStore";
import {
  DEFAULT_USER_AGENT_MIHOMO,
  DEFAULT_USER_AGENT_SINGBOX,
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
import { useUpdateStore } from "../stores/updateStore";
import { checkForUpdates } from "../lib/updater";
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
  /** i18n-ключ для заголовка категории. Резолвится через t() в месте рендера. */
  titleKey: string;
  /** i18n-ключ для описания категории. */
  descKey: string;
};

/** Метаданные категорий для рендера CategoryList. Иконки — эмодзи
 *  (без зависимости от иконочных шрифтов). Описание — короткая фраза
 *  что внутри, чтобы пользователь не открывал каждую наугад. */
const CATEGORIES: CategoryMeta[] = [
  {
    id: "subscription",
    icon: "📡",
    titleKey: "settings.categories.subscription.title",
    descKey: "settings.categories.subscription.desc",
  },
  {
    id: "connection",
    icon: "🔌",
    titleKey: "settings.categories.connection.title",
    descKey: "settings.categories.connection.desc",
  },
  {
    id: "engine",
    icon: "⚙️",
    titleKey: "settings.categories.engine.title",
    descKey: "settings.categories.engine.desc",
  },
  {
    id: "tunnel",
    icon: "🛡️",
    titleKey: "settings.categories.tunnel.title",
    descKey: "settings.categories.tunnel.desc",
  },
  {
    id: "security",
    icon: "🔒",
    titleKey: "settings.categories.security.title",
    descKey: "settings.categories.security.desc",
  },
  {
    id: "routing",
    icon: "🗺️",
    titleKey: "settings.categories.routing.title",
    descKey: "settings.categories.routing.desc",
  },
  {
    id: "appearance",
    icon: "🎨",
    titleKey: "settings.categories.appearance.title",
    descKey: "settings.categories.appearance.desc",
  },
  {
    id: "system",
    icon: "🔧",
    titleKey: "settings.categories.system.title",
    descKey: "settings.categories.system.desc",
  },
];

export function SettingsPage({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
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
  // sing-box миграция (0.1.2): legacy header "xray" автоматически
  // мапится в "sing-box".
  const headerEngineRaw = subMeta?.engine;
  const headerEngine: Engine | null =
    headerEngineRaw === "mihomo"
      ? "mihomo"
      : headerEngineRaw === "sing-box" || headerEngineRaw === "xray"
      ? "sing-box"
      : null;
  const effectiveEngine: Engine =
    !s.engineTouched && headerEngine ? headerEngine : s.engine;
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
      ? t("settings.title")
      : t(
          CATEGORIES.find((c) => c.id === category)?.titleKey ??
            "settings.title"
        ).toLowerCase();

  return (
    <div className="settings-page">
      <div className="settings-frame">
        <header className="settings-header">
          <button
            type="button"
            onClick={onBack}
            className="back-btn"
            aria-label={t("common.back")}
          >
            ← {t("common.back")}
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
                <div className="settings-section-title">{t("settings.subscription.title")}</div>
                {subMeta?.title && (
                  <div className="settings-row-hint" style={{ marginBottom: 8 }}>
                    {subMeta.title} <span className="hint-badge">{t("settings.fromSubscription")}</span>
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
                    {subLoading ? "…" : t("common.refresh")}
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
                    {t("settings.subscription.dashboard")}
                  </button>
                )}
              </section>

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.autoRefresh.title")}</div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.autoRefresh.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.autoRefresh.hint")}
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
                        {t("settings.autoRefresh.intervalHours")}
                        {!s.autoRefreshHoursTouched &&
                          subMeta?.updateIntervalHours != null && (
                            <span className="hint-badge" style={{ marginLeft: 8 }}>
                              {t("settings.fromSubscription")}
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
                <div className="settings-section-title">{t("settings.dataSending.title")}</div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.dataSending.sendHwid.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.dataSending.sendHwid.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.sendHwid}
                    onChange={(v) => s.set("sendHwid", v)}
                  />
                </div>

                <div className="settings-row" style={{ flexDirection: "column", alignItems: "stretch", gap: 6 }}>
                  <div className="settings-row-label">
                    {t("settings.dataSending.userAgent.label")}
                    {!s.userAgentTouched && (
                      <span className="hint-badge" style={{ marginLeft: 8 }}>
                        {t("settings.dataSending.userAgent.autoBadge")}
                      </span>
                    )}
                  </div>
                  <input
                    type="text"
                    value={s.userAgent}
                    onChange={(e) => s.set("userAgent", e.target.value)}
                    placeholder={mihomoActive ? DEFAULT_USER_AGENT_MIHOMO : DEFAULT_USER_AGENT_SINGBOX}
                    className="input"
                  />
                  <div className="settings-row-hint">
                    {t("settings.dataSending.userAgent.hint")}
                  </div>
                </div>
              </section>

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.hwid.title")}</div>
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
                    {hwidCopied ? t("common.ok") : t("common.copy")}
                  </button>
                </div>
                <p className="hint">
                  {t("settings.hwid.hint")}
                </p>

                <button
                  type="button"
                  onClick={() => setAdvancedOpen((v) => !v)}
                  className="advanced-toggle"
                >
                  {advancedOpen ? `▾ ${t("settings.hwid.override")}` : `▸ ${t("settings.hwid.override")}`}
                </button>
                {advancedOpen && (
                  <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 8 }}>
                    {subHwid.trim() && (
                      <div className="warn-box">
                        <span className="warn-box-text">
                          {t("settings.hwid.overrideActive", { value: subHwid.slice(0, 12) })}
                        </span>
                        <button
                          type="button"
                          onClick={() => setSubHwid("")}
                          className="btn-ghost"
                        >
                          {t("settings.hwid.resetOverride")}
                        </button>
                      </div>
                    )}
                    <input
                      type="text"
                      value={subHwid}
                      onChange={(e) => setSubHwid(e.target.value)}
                      placeholder={
                        deviceHwid || t("settings.hwid.placeholder")
                      }
                      className="input"
                    />
                  </div>
                )}
              </section>

              <ComingSoonNote
                titleKey="settings.comingSoon.mergeSubs.title"
                descKey="settings.comingSoon.mergeSubs.desc"
              />
            </>
          )}

          {/* ── Подключение ─────────────────────────────────────────────── */}
          {category === "connection" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">{t("settings.connection.onStart.title")}</div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.connection.refreshOnOpen.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.connection.refreshOnOpen.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.refreshOnOpen}
                    onChange={(v) => s.set("refreshOnOpen", v)}
                  />
                </div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.connection.pingOnOpen.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.connection.pingOnOpen.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.pingOnOpen}
                    onChange={(v) => s.set("pingOnOpen", v)}
                  />
                </div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.connection.connectOnOpen.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.connection.connectOnOpen.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.connectOnOpen}
                    onChange={(v) => s.set("connectOnOpen", v)}
                  />
                </div>
              </section>

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.connection.sort.title")}</div>
                {(
                  [
                    ["none", "settings.connection.sort.none"],
                    ["ping", "settings.connection.sort.ping"],
                    ["name", "settings.connection.sort.name"],
                  ] as [SortMode, string][]
                ).map(([value, labelKey]) => (
                  <label key={value} className="radio-row">
                    <input
                      type="radio"
                      name="sort"
                      checked={s.sort === value}
                      onChange={() => s.set("sort", value)}
                    />
                    <span>{t(labelKey)}</span>
                  </label>
                ))}
              </section>

              <ComingSoonNote
                titleKey="settings.comingSoon.autoFailover.title"
                descKey="settings.comingSoon.autoFailover.desc"
              />
            </>
          )}

          {/* ── Движок ──────────────────────────────────────────────────── */}
          {category === "engine" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">{t("settings.engine.title")}</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      {t("settings.engine.label")}
                      {!s.engineTouched && subMeta?.engine && (
                        <span className="hint-badge" style={{ marginLeft: 8 }}>
                          {t("settings.fromSubscription")}
                        </span>
                      )}
                    </div>
                    <div className="settings-row-hint">
                      {t("settings.engine.hint")}
                    </div>
                  </div>
                  <select
                    className="select-field"
                    value={effectiveEngine}
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
                    <option value="sing-box">sing-box</option>
                    <option value="mihomo">Mihomo</option>
                  </select>
                </div>
              </section>

              <AppRulesSection mihomoActive={mihomoActive} />
            </>
          )}

          {/* ── Туннель ─────────────────────────────────────────────────── */}
          {category === "tunnel" && (
            <>
              <section className="settings-section">
                <div className="settings-section-title">{t("settings.tunnel.network.title")}</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.tunnel.allowLan.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.tunnel.allowLan.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.allowLan}
                    onChange={(v) => s.set("allowLan", v)}
                  />
                </div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.tunnel.tunMasking.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.tunnel.tunMasking.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.tunMasking}
                    onChange={(v) => s.set("tunMasking", v)}
                  />
                </div>

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.tunnel.tunOnlyStrict.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.tunnel.tunOnlyStrict.hint")}
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
                  {t("settings.antiDpi.title")}
                  {!s.antiDpiTouched &&
                    (subMeta?.fragmentationEnable != null ||
                      subMeta?.noisesEnable != null ||
                      subMeta?.serverResolveEnable != null) && (
                      <span className="hint-badge" style={{ marginLeft: 8 }}>
                        {t("settings.fromSubscription")}
                      </span>
                    )}
                </div>

                {/* Anti-DPI имеет разный support по движкам.
                    sing-box: tls.fragment (boolean, без тонкой настройки size/sleep)
                    + DoH-резолв адреса сервера. UDP noises upstream sing-box
                    НЕ поддерживает.
                    mihomo: anti-DPI обвязка не реализована (DNS-resolve работает). */}
                {mihomoActive && (
                  <div className="hint-warning">
                    {t("settings.antiDpi.mihomoWarning")}
                  </div>
                )}
                {!mihomoActive && (
                  <div className="hint-info" style={{ marginBottom: 8 }}>
                    {t("settings.antiDpi.singboxInfo")}
                  </div>
                )}

                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.antiDpi.fragmentation.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.antiDpi.fragmentation.hint")}
                      {mihomoActive && t("settings.antiDpi.fragmentation.singboxOnly")}
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
                        <div className="settings-row-label">{t("settings.antiDpi.fragmentation.packetsLabel")}</div>
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
                        <option value="all">{t("settings.antiDpi.fragmentation.packetsAll")}</option>
                      </select>
                    </div>
                    <div className="settings-row">
                      <div>
                        <div className="settings-row-label">{t("settings.antiDpi.fragmentation.lengthLabel")}</div>
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
                        <div className="settings-row-label">{t("settings.antiDpi.fragmentation.intervalLabel")}</div>
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
                    <div className="settings-row-label">{t("settings.antiDpi.noises.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.antiDpi.noises.hint")}
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
                    <div className="settings-row-label">{t("settings.antiDpi.dohResolve.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.antiDpi.dohResolve.hint")}
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
                        <div className="settings-row-label">{t("settings.antiDpi.dohResolve.endpointLabel")}</div>
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
                        <div className="settings-row-label">{t("settings.antiDpi.dohResolve.bootstrapLabel")}</div>
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
                <div className="settings-section-title">{t("settings.killSwitch.title")}</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.killSwitch.main.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.killSwitch.main.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.killSwitch}
                    onChange={(v) => s.set("killSwitch", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.killSwitch.strict.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.killSwitch.strict.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.killSwitchStrict}
                    onChange={(v) => s.set("killSwitchStrict", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.killSwitch.dnsLeak.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.killSwitch.dnsLeak.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.dnsLeakProtection}
                    onChange={(v) => s.set("dnsLeakProtection", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.killSwitch.recover.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.killSwitch.recover.hint")}
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
                            r.kill_switch_cleaned ? t("toast.recover.parts.wfp") : null,
                            r.orphan_resources_cleaned
                              ? t("toast.recover.parts.tunRoutes")
                              : null,
                            r.system_proxy_cleared ? t("toast.recover.parts.proxy") : null,
                          ].filter(Boolean);
                          if (r.errors.length === 0) {
                            showToast({
                              kind: "success",
                              title: t("toast.recover.successTitle"),
                              message:
                                cleaned.length > 0
                                  ? t("toast.recover.cleaned", { items: cleaned.join(", ") })
                                  : t("toast.recover.nothingToClean"),
                            });
                          } else {
                            showToast({
                              kind: "warning",
                              title: t("toast.recover.partialTitle"),
                              message: `${
                                cleaned.length > 0
                                  ? t("toast.recover.okPrefix", { items: cleaned.join(", ") }) + "\n"
                                  : ""
                              }${t("toast.recover.errorsPrefix", { errors: r.errors.join("; ") })}`,
                              durationMs: 12_000,
                            });
                          }
                        }
                      );
                    }}
                  >
                    {t("settings.killSwitch.recover.button")}
                  </button>
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.diagnostics.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.diagnostics.hint")}
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
                            title: t("toast.diagnostics.savedTitle"),
                            message: t("toast.diagnostics.savedMessage", { path }),
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
                            title: t("toast.diagnostics.failedTitle"),
                            message: String(e),
                          })
                        );
                    }}
                  >
                    {t("settings.diagnostics.button")}
                  </button>
                </div>
              </section>

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.leakTest.title")}</div>
                <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5, marginBottom: 8 }}>
                  {t("settings.leakTest.description")}
                </p>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.leakTest.auto.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.leakTest.auto.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.autoLeakTest}
                    onChange={(v) => s.set("autoLeakTest", v)}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">{t("settings.leakTest.run.label")}</div>
                    <div className="settings-row-hint">
                      {t("settings.leakTest.run.hint")}
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
                    {t("settings.leakTest.run.button")}
                  </button>
                </div>
              </section>

              <ComingSoonNote
                titleKey="settings.comingSoon.windowsHello.title"
                descKey="settings.comingSoon.windowsHello.desc"
              />
            </>
          )}

          {/* ── Маршрутизация ───────────────────────────────────────────── */}
          {category === "routing" && (
            <>
              <div className="settings-row-hint" style={{ marginBottom: 12 }}>
                {t("settings.routing.intro")}
              </div>
              <RoutingProfilesPanel />
              <section className="settings-section">
                <div className="settings-section-title">{t("settings.routing.autoTemplate.title")}</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      {t("settings.routing.autoTemplate.label")}
                    </div>
                    <div className="settings-row-hint">
                      {t("settings.routing.autoTemplate.hint")}
                    </div>
                  </div>
                  <Toggle
                    on={s.autoApplyMinimalRuRules}
                    onChange={(v) => s.set("autoApplyMinimalRuRules", v)}
                  />
                </div>
              </section>
              <ComingSoonNote
                titleKey="settings.comingSoon.wfpPerApp.title"
                descKey="settings.comingSoon.wfpPerApp.desc"
              />
            </>
          )}

          {/* ── Интерфейс ───────────────────────────────────────────────── */}
          {category === "appearance" && (
            <>
              <LanguageSection />

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.appearance.preset.title")}</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      {t("settings.appearance.preset.label")}
                      {eff.fromSubscription.preset && (
                        <span className="hint-badge" style={{ marginLeft: 8 }}>
                          {t("settings.fromSubscription")}
                        </span>
                      )}
                    </div>
                    <div className="settings-row-hint">
                      {t("settings.appearance.preset.hint")}
                    </div>
                  </div>
                  <select
                    className="select-field"
                    value={eff.preset}
                    onChange={(e) => s.set("preset", e.target.value as Preset)}
                  >
                    <option value="none">{t("settings.appearance.preset.options.none")}</option>
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
                const presetHint = t("settings.appearance.themeStyle.presetHint");
                return (
                  <section className="settings-section">
                    <div className="settings-section-title">{t("settings.appearance.themeStyle.title")}</div>
                    <div className="settings-row">
                      <div>
                        <div className="settings-row-label">
                          {t("settings.appearance.theme.label")}
                          {!presetActive && eff.fromSubscription.theme && (
                            <span className="hint-badge" style={{ marginLeft: 8 }}>
                              {t("settings.fromSubscription")}
                            </span>
                          )}
                        </div>
                        <div className="settings-row-hint">
                          {presetActive ? presetHint : t("settings.appearance.theme.hint")}
                        </div>
                      </div>
                      <select
                        className="select-field"
                        value={eff.theme}
                        disabled={presetActive}
                        onChange={(e) => s.set("theme", e.target.value as Theme)}
                      >
                        <option value="dark">{t("settings.appearance.theme.options.dark")}</option>
                        <option value="light">{t("settings.appearance.theme.options.light")}</option>
                        <option value="midnight">midnight</option>
                        <option value="sunset">sunset</option>
                        <option value="sand">sand</option>
                      </select>
                    </div>
                    <div className="settings-row">
                      <div>
                        <div className="settings-row-label">
                          {t("settings.appearance.background.label")}
                          {!presetActive && eff.fromSubscription.background && (
                            <span className="hint-badge" style={{ marginLeft: 8 }}>
                              {t("settings.fromSubscription")}
                            </span>
                          )}
                        </div>
                        <div className="settings-row-hint">
                          {presetActive ? presetHint : t("settings.appearance.background.hint")}
                        </div>
                      </div>
                      <select
                        className="select-field"
                        value={effectiveBg}
                        disabled={presetActive}
                        onChange={(e) => s.set("background", e.target.value as Background)}
                      >
                        <option value="crystal">{t("settings.appearance.background.options.crystal")}</option>
                        <option value="tunnel">{t("settings.appearance.background.options.tunnel")}</option>
                        <option value="globe">{t("settings.appearance.background.options.globe")}</option>
                        <option value="particles">{t("settings.appearance.background.options.particles")}</option>
                      </select>
                    </div>
                    <div className="settings-row">
                      <div>
                        <div className="settings-row-label">
                          {t("settings.appearance.buttonStyle.label")}
                          {!presetActive && eff.fromSubscription.buttonStyle && (
                            <span className="hint-badge" style={{ marginLeft: 8 }}>
                              {t("settings.fromSubscription")}
                            </span>
                          )}
                        </div>
                        <div className="settings-row-hint">
                          {presetActive ? presetHint : t("settings.appearance.buttonStyle.hint")}
                        </div>
                      </div>
                      <select
                        className="select-field"
                        value={effectiveStyle}
                        disabled={presetActive}
                        onChange={(e) => s.set("buttonStyle", e.target.value as ButtonStyle)}
                      >
                        <option value="glass">{t("settings.appearance.buttonStyle.options.glass")}</option>
                        <option value="flat">{t("settings.appearance.buttonStyle.options.flat")}</option>
                        <option value="neon">{t("settings.appearance.buttonStyle.options.neon")}</option>
                        <option value="metallic">{t("settings.appearance.buttonStyle.options.metallic")}</option>
                      </select>
                    </div>
                  </section>
                );
              })()}

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.appearance.floating.title")}</div>
                <div className="settings-row">
                  <div>
                    <div className="settings-row-label">
                      {t("settings.appearance.floating.label")}
                    </div>
                    <div className="settings-row-hint">
                      {t("settings.appearance.floating.hint")}
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
                <div className="settings-section-title">{t("settings.system.autostart.title")}</div>
                <AutostartRow />
              </section>

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.shortcuts.title")}</div>
                <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5, marginBottom: 8 }}>
                  {t("settings.shortcuts.intro")}
                </p>
                <ShortcutInput
                  label={t("settings.shortcuts.toggleVpn.label")}
                  hint={t("settings.shortcuts.toggleVpn.hint")}
                  value={s.shortcutToggleVpn}
                  onChange={(v) => s.set("shortcutToggleVpn", v)}
                />
                <ShortcutInput
                  label={t("settings.shortcuts.showHide.label")}
                  hint={t("settings.shortcuts.showHide.hint")}
                  value={s.shortcutShowHide}
                  onChange={(v) => s.set("shortcutShowHide", v)}
                />
                <ShortcutInput
                  label={t("settings.shortcuts.switchMode.label")}
                  hint={t("settings.shortcuts.switchMode.hint")}
                  value={s.shortcutSwitchMode}
                  onChange={(v) => s.set("shortcutSwitchMode", v)}
                />
              </section>

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.trustedWifi.title")}</div>
                <TrustedWifiBlock />
              </section>

              <ComingSoonNote
                titleKey="settings.comingSoon.autoUpdate.title"
                descKey="settings.comingSoon.autoUpdate.desc"
              />
              <ComingSoonNote
                titleKey="settings.comingSoon.history.title"
                descKey="settings.comingSoon.history.desc"
              />
              <ComingSoonNote
                titleKey="settings.comingSoon.speedTest.title"
                descKey="settings.comingSoon.speedTest.desc"
              />
              <BackupBlock />

              <LogsBlock />

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.urlSchemes.title")}</div>
                <p className="hint" style={{ textTransform: "none", letterSpacing: 0, color: "var(--fg-dim)", fontSize: 12, lineHeight: 1.5 }}>
                  {t("settings.urlSchemes.intro")}
                </p>
                <div className="schemes">
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://add?url=&lt;url&gt;</span>
                    <span className="scheme-desc">{t("settings.urlSchemes.add")}</span>
                  </div>
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://connect</span>
                    <span className="scheme-desc">{t("settings.urlSchemes.connect")}</span>
                  </div>
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://disconnect</span>
                    <span className="scheme-desc">{t("settings.urlSchemes.disconnect")}</span>
                  </div>
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://toggle</span>
                    <span className="scheme-desc">{t("settings.urlSchemes.toggle")}</span>
                  </div>
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://export</span>
                    <span className="scheme-desc">{t("settings.urlSchemes.export")}</span>
                  </div>
                  <div className="scheme-row">
                    <span className="scheme-url">nemefisto://import-from-url/&lt;url&gt;</span>
                    <span className="scheme-desc">{t("settings.urlSchemes.importFromUrl")}</span>
                  </div>
                </div>
              </section>

              <UpdatesSection />

              <section className="settings-section">
                <div className="settings-section-title">{t("settings.about.title")}</div>
                <div className="about-grid">
                  <span className="about-key">{t("settings.about.version")}</span>
                  <span className="about-val">v.{APP_VERSION} · build 2026.4</span>
                  <span className="about-key">sing-box</span>
                  <span className="about-val">1.13.x</span>
                  <span className="about-key">mihomo</span>
                  <span className="about-val">v1.19.24</span>
                  {subMeta?.webPageUrl && (
                    <>
                      <span className="about-key">{t("settings.about.dashboard")}</span>
                      <button
                        type="button"
                        onClick={openDashboard}
                        className="about-link"
                      >
                        {(() => {
                          try {
                            return new URL(subMeta.webPageUrl).host;
                          } catch {
                            return t("settings.about.link");
                          }
                        })()}
                      </button>
                    </>
                  )}
                  <span className="about-key">{t("settings.about.support")}</span>
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
                  <span className="about-key">{t("settings.about.privacy")}</span>
                  <button
                    type="button"
                    onClick={() => void openUrl(PRIVACY_URL)}
                    className="about-link"
                  >
                    PRIVACY.md
                  </button>
                  <span className="about-key">{t("settings.about.license")}</span>
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
                  {t("settings.about.privacyNote")}
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
  const { t } = useTranslation();
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
            <span className="settings-category-title">{t(c.titleKey)}</span>
            <span className="settings-category-desc">{t(c.descKey)}</span>
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

function ComingSoonNote({ titleKey, descKey }: { titleKey: string; descKey: string }) {
  const { t } = useTranslation();
  return (
    <section className="settings-section coming-soon">
      <div className="coming-soon-row">
        <span className="coming-soon-badge">{t("settings.comingSoon.badge")}</span>
        <div className="coming-soon-text">
          <div className="coming-soon-title">{t(titleKey)}</div>
          <div className="coming-soon-desc">{t(descKey)}</div>
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
 * Mihomo нативно умеет PROCESS-NAME matcher; sing-box на Windows — нет
 * (рассматривается через WFP в этапе 13.G). Если активен sing-box —
 * баннер сверху предупреждает что правила игнорируются. Хранятся всегда —
 * при переключении движка на Mihomo сразу применятся.
 */
function AppRulesSection({ mihomoActive }: { mihomoActive: boolean }) {
  const { t } = useTranslation();
  const rules = useSettingsStore((s) => s.appRules);
  const set = useSettingsStore((s) => s.set);
  // 8.D: PROCESS-NAME matcher Mihomo на Windows работает в двух
  // случаях: (1) proxy-режим — приложение коннектится напрямую к
  // mixed-inbound Mihomo; (2) TUN-режим с mihomo-profile подпиской
  // (Mihomo built-in TUN через WinTUN, helper SYSTEM-spawn) — Mihomo
  // сам владеет адаптером и видит ядерный PID. Для URI-серверов
  // (vless/vmess/...) в TUN-режиме всё ещё используется tun2proxy
  // sidecar pipeline — там Mihomo видит PID tun2proxy, не исходного
  // приложения, matcher не срабатывает.
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
      <div className="settings-section-title">{t("settings.appRules.title")}</div>

      {!mihomoActive && (
        <div className="hint-warning">
          {t("settings.appRules.singboxWarning")}
        </div>
      )}

      {mihomoActive && tunMode && (
        <div className="hint-warning">
          {t("settings.appRules.tunWarning")}
        </div>
      )}

      <div className="settings-row-hint" style={{ marginBottom: 10 }}>
        {t("settings.appRules.intro")}
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
                    ? t("settings.appRules.actionTitles.proxy")
                    : r.action === "direct"
                    ? t("settings.appRules.actionTitles.direct")
                    : t("settings.appRules.actionTitles.block")
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
                title={t("settings.appRules.deleteTitle")}
                aria-label={t("common.delete")}
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
          placeholder={t("settings.appRules.commentPlaceholder")}
          onKeyDown={(e) => e.key === "Enter" && addRule()}
        />
        <button
          type="button"
          className="btn-ghost"
          onClick={addRule}
          disabled={!draftExe.trim()}
        >
          {t("common.add")}
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
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  const onExport = async () => {
    setBusy(true);
    try {
      const path = await exportBackupToDocuments();
      showToast({
        kind: "success",
        title: t("toast.backup.exportedTitle"),
        message: path,
        durationMs: 8000,
      });
    } catch (e) {
      showToast({ kind: "error", title: t("toast.backup.exportFailed"), message: String(e) });
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
          title: t("toast.backup.readFailed"),
          message: String(err),
        });
      });
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">{t("settings.backup.title")}</div>
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
        {t("settings.backup.intro")}
      </p>
      <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <button
          type="button"
          onClick={onExport}
          disabled={busy}
          className="btn-ghost"
        >
          {t("settings.backup.export")}
        </button>
        <label className="btn-ghost" style={{ cursor: "pointer" }}>
          {t("settings.backup.import")}
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

// ── 14.J Language section ─────────────────────────────────────────────────

function LanguageSection() {
  const language = useSettingsStore((s) => s.language);
  const setSetting = useSettingsStore((s) => s.set);
  const { i18n, t } = useTranslation();

  const onChange = (value: "auto" | "ru" | "en") => {
    setSetting("language", value);
    // i18n.changeLanguage:
    // - "auto" → детектим из navigator.language
    // - "ru" / "en" → явный
    if (value === "auto") {
      const nav = navigator.language?.toLowerCase() ?? "";
      void i18n.changeLanguage(nav.startsWith("ru") ? "ru" : "en");
    } else {
      void i18n.changeLanguage(value);
    }
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">{t("settings.language.title")}</div>
      <div className="settings-row">
        <div>
          <div className="settings-row-label">{t("settings.language.label")}</div>
          <div className="settings-row-hint">
            {t("settings.language.hint")}
          </div>
        </div>
        <select
          className="select-field"
          value={language}
          onChange={(e) =>
            onChange(e.target.value as "auto" | "ru" | "en")
          }
        >
          <option value="auto">{t("settings.language.auto")}</option>
          <option value="ru">Русский</option>
          <option value="en">English</option>
        </select>
      </div>
    </section>
  );
}

// ── 14.A Updates section ──────────────────────────────────────────────────

function UpdatesSection() {
  const { t } = useTranslation();
  const autoCheck = useSettingsStore((s) => s.autoCheckUpdates);
  const setSetting = useSettingsStore((s) => s.set);
  const updateState = useUpdateStore((s) => s.state);
  const setUpdateState = useUpdateStore((s) => s.setState);
  const setLastCheckAt = useUpdateStore((s) => s.setLastCheckAt);
  const [busy, setBusy] = useState(false);

  const onCheckNow = async () => {
    setBusy(true);
    setUpdateState({ kind: "checking" });
    setLastCheckAt(Date.now());
    try {
      const update = await checkForUpdates();
      if (update) {
        setUpdateState({ kind: "available", update });
      } else {
        setUpdateState({ kind: "idle" });
        showToast({
          kind: "success",
          title: t("toast.update.checkTitle"),
          message: t("toast.update.upToDate"),
        });
      }
    } catch (e) {
      setUpdateState({ kind: "idle" });
      showToast({
        kind: "error",
        title: t("toast.update.checkFailed"),
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const checking = updateState.kind === "checking" || busy;

  return (
    <section className="settings-section">
      <div className="settings-section-title">{t("settings.updates.title")}</div>
      <div className="settings-row">
        <div>
          <div className="settings-row-label">{t("settings.updates.auto.label")}</div>
          <div className="settings-row-hint">
            {t("settings.updates.auto.hint")}
          </div>
        </div>
        <Toggle
          on={autoCheck}
          onChange={(v) => setSetting("autoCheckUpdates", v)}
        />
      </div>
      <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 8 }}>
        <button
          type="button"
          onClick={onCheckNow}
          disabled={checking}
          className="btn-ghost"
        >
          {checking ? t("settings.updates.checking") : t("settings.updates.checkNow")}
        </button>
      </div>
    </section>
  );
}

// ── Logs block ────────────────────────────────────────────────────────────

function LogsBlock() {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [loaded, setLoaded] = useState(false);

  const reload = async () => {
    try {
      const log = await invoke<string>("read_xray_log");
      setText(log || t("settings.logs.empty"));
      setLoaded(true);
    } catch (e) {
      setText(String(e));
      setLoaded(true);
    }
  };

  return (
    <section className="settings-section">
      <div className="settings-section-title">{t("settings.logs.title")}</div>
      {!loaded ? (
        <button
          type="button"
          onClick={reload}
          className="btn-ghost"
          style={{ alignSelf: "flex-start" }}
        >
          {t("settings.logs.show")}
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
            {t("common.refresh")}
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
  const { t } = useTranslation();
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
      <div className="settings-section-title">{t("settings.reset.title")}</div>

      {pending === null && (
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <button
            type="button"
            onClick={() => setPending("settings")}
            className="btn-ghost"
          >
            {t("settings.reset.resetSettings")}
          </button>
          <button
            type="button"
            onClick={() => setPending("all")}
            className="btn-danger"
          >
            {t("settings.reset.deleteAll")}
          </button>
        </div>
      )}

      {pending === "settings" && (
        <div className="warn-box" style={{ borderColor: "rgba(217,119,87,0.4)" }}>
          <span className="warn-box-text">
            {t("settings.reset.confirmSettings")}
          </span>
          <button
            type="button"
            onClick={() => setPending(null)}
            className="btn-ghost"
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            onClick={doResetSettings}
            className="btn-danger"
          >
            {t("settings.reset.confirmSettingsBtn")}
          </button>
        </div>
      )}

      {pending === "all" && (
        <div className="warn-box" style={{ borderColor: "rgba(217,119,87,0.6)" }}>
          <span className="warn-box-text">
            {t("settings.reset.confirmAll")}
          </span>
          <button
            type="button"
            onClick={() => setPending(null)}
            className="btn-ghost"
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            onClick={doResetAll}
            className="btn-danger"
          >
            {t("settings.reset.confirmAllBtn")}
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
  const { t } = useTranslation();
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
        <div className="settings-row-label">{t("settings.system.autostart.label")}</div>
        <div className="settings-row-hint">
          {t("settings.system.autostart.hint")}
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
  const { t } = useTranslation();
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
          ? t("settings.shortcuts.pressCombo")
          : value ?? t("settings.shortcuts.notSet")}
        {!recording && value && (
          <button
            type="button"
            className="shortcut-clear"
            onClick={(e) => {
              e.stopPropagation();
              onChange(null);
            }}
            title={t("settings.shortcuts.clear")}
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
  const { t } = useTranslation();
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
        {t("settings.trustedWifi.intro")}
      </p>

      <div className="trusted-current">
        <span className="trusted-current-label">{t("settings.trustedWifi.currentNetwork")}</span>
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
            {t("settings.trustedWifi.addThis")}
          </button>
        )}
        {isCurrentInList && (
          <span className="trusted-current-badge">{t("settings.trustedWifi.inList")}</span>
        )}
      </div>

      {trustedSsids.length > 0 && (
        <div className="app-rules-list" style={{ marginTop: 10 }}>
          {trustedSsids.map((ssid) => (
            <div key={ssid} className="app-rule-row">
              <span className="app-rule-exe">{ssid}</span>
              {ssid === currentSsid && (
                <span className="trusted-current-badge">{t("settings.trustedWifi.current")}</span>
              )}
              <button
                type="button"
                className="app-rule-del"
                onClick={() => removeSsid(ssid)}
                title={t("common.delete")}
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
          placeholder={t("settings.trustedWifi.manualPlaceholder")}
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
          {t("common.add")}
        </button>
      </div>

      <div className="settings-row" style={{ marginTop: 12 }}>
        <div>
          <div className="settings-row-label">{t("settings.trustedWifi.action.label")}</div>
          <div className="settings-row-hint">
            {t("settings.trustedWifi.action.hint")}
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
          <option value="ignore">{t("settings.trustedWifi.action.ignore")}</option>
          <option value="disconnect">{t("settings.trustedWifi.action.disconnect")}</option>
        </select>
      </div>

      <div className="settings-row">
        <div>
          <div className="settings-row-label">{t("settings.trustedWifi.autoLeave.label")}</div>
          <div className="settings-row-hint">
            {t("settings.trustedWifi.autoLeave.hint")}
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
