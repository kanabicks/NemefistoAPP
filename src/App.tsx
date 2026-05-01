import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import { useVpnStore } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";
import { useSettingsStore } from "./stores/settingsStore";
import { useApplyTheme } from "./lib/hooks/useApplyTheme";
import { useGlobalShortcuts } from "./lib/hooks/useGlobalShortcuts";
import { useTrustedWifi } from "./lib/hooks/useTrustedWifi";
import { initDeepLinks } from "./lib/deepLinks";

import { BackgroundLayers } from "./components/effects/BackgroundLayers";
import { Scene3D } from "./components/effects/Scene3D";
import { CustomCursor } from "./components/effects/CustomCursor";
import { WideAmbient } from "./components/effects/WideAmbient";
import { AnnounceBanner } from "./components/AnnounceBanner";
import { CrashRecoveryDialog } from "./components/CrashRecoveryDialog";
import { BackupPreviewModal } from "./components/BackupPreviewModal";
import {
  OnboardingTour,
  isOnboardingCompleted,
} from "./components/OnboardingTour";
import { ProxiesPanel } from "./components/ProxiesPanel";
import { useBackupModalStore } from "./lib/backup";
import { Header } from "./components/Header";
import { PowerStack } from "./components/PowerStack";
import { Welcome } from "./components/Welcome";
import { ServerSelector } from "./components/ServerSelector";
import { BandwidthMeter } from "./components/BandwidthMeter";
import { SubscriptionMeta } from "./components/SubscriptionMeta";
import { Toaster } from "./components/Toaster";
import { runLeakTest } from "./lib/leakTest";
import { ModeSegment } from "./components/ModeSegment";
import { Footer } from "./components/Footer";
import { SettingsPage } from "./components/SettingsPage";
import { openDashboard, useHasDashboardUrl } from "./lib/openExternal";

/**
 * Корневой компонент. Координирует:
 * - инициализацию stores при mount (refresh status, кеш, hwid, on-open actions);
 * - подписку на deep-links (nemefisto://...);
 * - авто-подключение к последнему серверу при старте (если включено);
 * - фоновый авто-refresh подписки.
 *
 * UI разбит на компоненты под `src/components/`. Каждый сам читает
 * нужные кусочки store'ов.
 */
function App() {
  // VPN status / mode
  const status = useVpnStore((s) => s.status);
  const errorMessage = useVpnStore((s) => s.errorMessage);
  const mode = useVpnStore((s) => s.mode);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const setMode = useVpnStore((s) => s.setMode);
  const connect = useVpnStore((s) => s.connect);
  const refresh = useVpnStore((s) => s.refresh);

  // Подписка
  const servers = useSubscriptionStore((s) => s.servers);
  const subscriptionMeta = useSubscriptionStore((s) => s.meta);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  const loadCached = useSubscriptionStore((s) => s.loadCached);
  const loadDeviceHwid = useSubscriptionStore((s) => s.loadDeviceHwid);
  const loadSecureCreds = useSubscriptionStore((s) => s.loadSecureCreds);
  const pingAll = useSubscriptionStore((s) => s.pingAll);

  // Settings
  const refreshOnOpen = useSettingsStore((x) => x.refreshOnOpen);
  const pingOnOpen = useSettingsStore((x) => x.pingOnOpen);
  const connectOnOpenSetting = useSettingsStore((x) => x.connectOnOpen);
  const autoRefresh = useSettingsStore((x) => x.autoRefresh);
  const autoRefreshHours = useSettingsStore((x) => x.autoRefreshHours);
  const autoRefreshHoursTouched = useSettingsStore(
    (x) => x.autoRefreshHoursTouched
  );
  const floatingWindow = useSettingsStore((x) => x.floatingWindow);
  const autoLeakTest = useSettingsStore((x) => x.autoLeakTest);
  const tunOnlyStrict = useSettingsStore((x) => x.tunOnlyStrict);
  const setSetting = useSettingsStore((x) => x.set);
  // Кнопка «личный кабинет» показывается только когда подписка
  // прислала `profile-web-page-url` (захардкоженный fallback убран).
  const hasDashboardUrl = useHasDashboardUrl();
  // 8.F: показать ProxiesPanel когда подключён mihomo-profile.
  const [proxiesPanelOpen, setProxiesPanelOpen] = useState(false);
  const engine = useSettingsStore((x) => x.engine);
  const isRunningStatus = status === "running";
  const selectedServer =
    selectedIndex !== null ? servers[selectedIndex] : null;
  const isMihomoProfile = selectedServer?.protocol === "mihomo-profile";
  const showProxiesPanelButton =
    isRunningStatus && engine === "mihomo" && isMihomoProfile;
  const socksPort = useVpnStore((s) => s.socksPort);

  const [settingsOpen, setSettingsOpen] = useState(false);

  // Применяем активную тему (data-theme на <html>). См. App.css :root[data-theme="light"].
  useApplyTheme();
  // 13.N: глобальные горячие клавиши. Регистрация/перерегистрация
  // отслеживается внутри хука по изменению settings.shortcut*.
  useGlobalShortcuts();
  // 13.M: отслеживаем trusted Wi-Fi сети — реакция на `wifi-changed`
  // event'ы из бэка. Хук сам читает settings.trustedSsids и
  // autoDisconnectedBySsid runtime-флаг.
  useTrustedWifi();

  // 13.R: TUN-only strict mode. Если пользователь только что включил
  // toggle и на главном экране был выбран proxy-режим — авто-переключаем
  // на tun. Иначе ModeSegment скрыт, и пользователь не может вручную
  // вернуться к proxy. Эффект — на изменение tunOnlyStrict, а не на
  // mount, чтобы не сбрасывать сохранённый proxy-режим при выключенном
  // toggle.
  useEffect(() => {
    if (tunOnlyStrict && mode === "proxy") {
      setMode("tun");
    }
  }, [tunOnlyStrict, mode, setMode]);

  // ── Старт: refresh статуса VPN, кеш списка, HWID, on-open actions ─────────
  useEffect(() => {
    refresh();
    loadCached();
    loadDeviceHwid();
    // Этап 6.A: подтягиваем URL/HWID из Windows Credential Manager
    // (с миграцией из localStorage при первом запуске). Делаем до
    // refreshOnOpen, чтобы fetchSubscription использовал актуальный URL.
    void loadSecureCreds().then(() => {
      if (refreshOnOpen) {
        void fetchSubscription();
      } else if (pingOnOpen) {
        void pingAll();
      }
    });

    // Подписка на deep-link события (nemefisto://add | connect | ...)
    let unlisten: (() => void) | undefined;
    initDeepLinks().then((u) => {
      unlisten = u;
    });

    // 14.C: один раз на старте проверяем количество свежих crash-dump'ов
    // (за последние 7 дней). Если есть — показываем мягкий toast с
    // подсказкой выгрузить диагностику для саппорта.
    void invoke<number>("count_recent_crashes")
      .then((count) => {
        if (count > 0) {
          import("./stores/toastStore").then(({ showToast }) => {
            showToast({
              kind: "warning",
              title: "обнаружен прошлый сбой",
              message: `найдено ${count} crash-dump'ов за неделю. в Settings → системное → диагностика можно собрать zip для саппорта`,
              durationMs: 12000,
            });
          });
        }
      })
      .catch(() => {});

    // 6.C: слушаем смену сетевого окружения. Если VPN был активен —
    // делаем reconnect: маршруты и xray sockopt.interface привязаны к
    // прежнему интерфейсу, после смены трафик не доходит. Reconnect
    // на свежем default-route чинит это автоматически.
    let unlistenNetwork: (() => void) | undefined;
    void listen<{ from: string | null; to: string | null }>(
      "network-changed",
      async (event) => {
        const v = useVpnStore.getState();
        if (v.status === "running") {
          console.log("[network-watcher] reconnect:", event.payload);
          await v.disconnect();
          // Маленькая пауза чтобы старые маршруты помылись и
          // platform::network успел отдать новый интерфейс.
          await new Promise((r) => setTimeout(r, 800));
          await v.connect();
        }
      }
    ).then((fn) => {
      unlistenNetwork = fn;
    });

    // 13.O: если у пользователя включено плавающее окно — показываем
    // его при старте. Окно создаётся в Rust setup всегда (скрытым),
    // здесь только .show().
    if (floatingWindow) {
      void invoke("show_floating_window");
    }
    // 13.O: пользователь нажал × на плавающем окне → бэкенд скрыл его
    // и эмитит `floating-closed`. Снимаем галку в settings чтобы при
    // следующем старте оно не появилось снова.
    let unlistenFloat: (() => void) | undefined;
    void listen("floating-closed", () => {
      setSetting("floatingWindow", false);
    }).then((fn) => {
      unlistenFloat = fn;
    });

    // 13.A: tray menu делегирует «toggle VPN» в фронт через event
    // `tray-action`. Здесь всю логику уже знает vpnStore (engine
    // selection, anti-DPI, kill-switch и т.д.), не дублируем на бэкенде.
    let unlistenTray: (() => void) | undefined;
    void listen<string>("tray-action", async (event) => {
      if (event.payload === "toggle-vpn") {
        const v = useVpnStore.getState();
        if (v.status === "running") {
          await v.disconnect();
        } else if (v.status === "stopped" || v.status === "error") {
          if (v.selectedIndex !== null) {
            await v.connect();
          }
        }
        // starting / stopping — игнорируем клик чтобы не дёргать.
      }
    }).then((fn) => {
      unlistenTray = fn;
    });

    return () => {
      unlisten?.();
      unlistenNetwork?.();
      unlistenTray?.();
      unlistenFloat?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // только один раз на mount

  // 13.A: при любом изменении статуса / выбранного сервера обновляем
  // tray (текст пункта «Подключить/Отключить» + tooltip иконки).
  useEffect(() => {
    const serverName =
      selectedIndex !== null && servers[selectedIndex]
        ? servers[selectedIndex].name
        : null;
    void invoke("tray_set_status", {
      status,
      serverName,
      hasSelection: selectedIndex !== null,
    });
  }, [status, selectedIndex, servers]);

  // 13.B/13.H: после успешного connect — авто-проверка IP/DNS leak.
  // Задержка 1.5 сек чтобы туннель устаканился (REALITY handshake,
  // прогрев, и т.п.). В TUN-режиме передаём null (через system route),
  // в proxy-режиме — наш SOCKS5 порт.
  useEffect(() => {
    if (status !== "running") return;
    if (!autoLeakTest) return;
    const port = mode === "proxy" ? socksPort : null;
    const timer = window.setTimeout(() => {
      void runLeakTest(port);
    }, 1500);
    return () => window.clearTimeout(timer);
  }, [status, autoLeakTest, mode, socksPort]);

  // 13.D: heartbeat для kill-switch watchdog. Пока vpn running и
  // killSwitch включён — пингуем helper каждые 20 сек. Если main
  // зависнет / упадёт — heartbeats перестанут идти, и helper через
  // ≤60 сек автоматически снимет фильтры (страховка от orphan'ов
  // даже если DYNAMIC session не сработала).
  const killSwitchEnabled = useSettingsStore((x) => x.killSwitch);
  useEffect(() => {
    if (status !== "running") return;
    if (!killSwitchEnabled) return;
    // Сразу первый heartbeat — чтобы watchdog был «прогрет» с момента 0.
    void invoke("kill_switch_heartbeat").catch(() => {});
    const id = window.setInterval(() => {
      void invoke("kill_switch_heartbeat").catch(() => {});
    }, 20_000);
    return () => window.clearInterval(id);
  }, [status, killSwitchEnabled]);

  // 13.D + 13.S: live-toggle kill-switch (включение/выключение и
  // strict-режим) без disconnect/connect. При активном VPN
  // пользователь в Settings меняет переключатели — реактивно применяем
  // через `kill_switch_apply`. Параметры (server_ips, app-paths, dns)
  // Rust берёт из контекста, сохранённого в connect; strict передаём
  // явно — backend обновит контекст перед re-apply.
  //
  // Через useRef отличаем «первый рендер с уже включённым» (connect
  // сам всё применил) от «user toggle» — без этого при каждом connect
  // дёргалась бы лишняя re-apply.
  const killSwitchStrictEnabled = useSettingsStore((x) => x.killSwitchStrict);
  const prevKillSwitch = useRef(killSwitchEnabled);
  const prevKillSwitchStrict = useRef(killSwitchStrictEnabled);
  useEffect(() => {
    if (status !== "running") {
      prevKillSwitch.current = killSwitchEnabled;
      prevKillSwitchStrict.current = killSwitchStrictEnabled;
      return;
    }
    const enabledChanged = prevKillSwitch.current !== killSwitchEnabled;
    const strictChanged =
      prevKillSwitchStrict.current !== killSwitchStrictEnabled;
    if (!enabledChanged && !strictChanged) return;
    prevKillSwitch.current = killSwitchEnabled;
    prevKillSwitchStrict.current = killSwitchStrictEnabled;
    void invoke("kill_switch_apply", {
      enabled: killSwitchEnabled,
      strict: killSwitchStrictEnabled,
    }).catch((e) => console.error("[kill_switch_apply]", e));
  }, [status, killSwitchEnabled, killSwitchStrictEnabled]);

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
  // Override-логика 8.C: если пользователь сам не трогал интервал, используем
  // значение из заголовка `profile-update-interval` подписки. Иначе — юзер-
  // настройку.
  const effectiveRefreshHours =
    !autoRefreshHoursTouched && subscriptionMeta?.updateIntervalHours
      ? subscriptionMeta.updateIntervalHours
      : autoRefreshHours;
  useEffect(() => {
    if (!autoRefresh) return;
    const ms = Math.max(1, effectiveRefreshHours) * 3600 * 1000;
    const id = window.setInterval(() => {
      void fetchSubscription();
    }, ms);
    return () => window.clearInterval(id);
  }, [autoRefresh, effectiveRefreshHours, fetchSubscription]);

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";
  const canConnect = selectedIndex !== null && !isBusy;

  return (
    <>
      <BackgroundLayers />
      <Scene3D status={status} />
      <WideAmbient />
      <CustomCursor />

      <div className="app">
        <div className="frame">
          <AnnounceBanner />
          <Header onOpenSettings={() => setSettingsOpen(true)} />

          {/* main-grid:
              - на узких — flex column в порядке
                power → servers/welcome → error → mode-seg;
              - на широких (≥1024px) — две колонки через grid-template-areas:
                слева power+mode, справа постоянно открытый server-list. */}
          <div className="main-grid">
            <div className="grid-power">
              <PowerStack canConnect={canConnect} />
            </div>
            <div className="grid-servers">
              {servers.length === 0 ? (
                <Welcome />
              ) : (
                <>
                  <SubscriptionMeta />
                  <ServerSelector />
                  <BandwidthMeter />
                </>
              )}
            </div>
            {errorMessage && (
              <pre className="hero-error grid-error">{errorMessage}</pre>
            )}
            {/* ModeSegment скрыт пока подписка не добавлена — переключать
                режим прокси/tun без серверов смысла нет, и Welcome card
                с инструкцией читается чище без лишних элементов. 13.R:
                при tunOnlyStrict выбор режима прячем — работает только
                TUN, useEffect выше уже гарантирует mode === "tun". */}
            {servers.length > 0 && !tunOnlyStrict && (
              <div className="grid-mode">
                <ModeSegment
                  mode={mode}
                  onChange={setMode}
                  disabled={isRunning || isBusy}
                />
              </div>
            )}
          </div>

          {/* Быстрый доступ в личный кабинет с главного экрана.
              Скрываем когда:
                - показан Welcome (там своя кнопка),
                - подписка не прислала `profile-web-page-url` (нет URL —
                  нет кнопки, см. openExternal.ts). */}
          {servers.length > 0 && hasDashboardUrl && (
            <button
              type="button"
              onClick={openDashboard}
              className="dashboard-link"
            >
              <span>личный кабинет</span>
              <span className="dashboard-link-arrow">→</span>
            </button>
          )}

          {/* 8.F: переход в ProxiesPanel — только когда активен
              mihomo-profile (full YAML с proxy-groups). Показывается под
              dashboard-link или вместо него. */}
          {showProxiesPanelButton && (
            <button
              type="button"
              onClick={() => setProxiesPanelOpen(true)}
              className="dashboard-link"
            >
              <span>прокси-группы</span>
              <span className="dashboard-link-arrow">→</span>
            </button>
          )}

          <Footer />
        </div>
      </div>

      {settingsOpen && (
        <SettingsPage onClose={() => setSettingsOpen(false)} />
      )}

      <CrashRecoveryDialog />
      <BackupPreview />
      <OnboardingHost />
      {proxiesPanelOpen && (
        <ProxiesPanel onClose={() => setProxiesPanelOpen(false)} />
      )}
      <Toaster />
    </>
  );
}

/** 14.G — first-run onboarding. Показывается ровно один раз — после
 *  пройденного шага флаг сохраняется в localStorage. Не показываем,
 *  если у пользователя уже есть кешированные серверы (значит он уже
 *  использовал приложение раньше — даже без флага онбординг бесполезен). */
function OnboardingHost() {
  const servers = useSubscriptionStore((s) => s.servers);
  const [open, setOpen] = useState(() => {
    if (isOnboardingCompleted()) return false;
    if (servers.length > 0) return false;
    return true;
  });
  if (!open) return null;
  return <OnboardingTour onClose={() => setOpen(false)} />;
}

/** 12.D — рендерит preview-модалку, когда deep-link или кнопка
 *  «импорт» положили backup в `useBackupModalStore.pending`. */
function BackupPreview() {
  const pending = useBackupModalStore((s) => s.pending);
  const close = useBackupModalStore((s) => s.close);
  if (!pending) return null;
  return <BackupPreviewModal backup={pending} onClose={close} />;
}

export default App;
