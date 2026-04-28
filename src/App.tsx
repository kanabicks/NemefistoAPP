import { useEffect, useState } from "react";
import "./App.css";
import { useVpnStore } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";
import { useSettingsStore } from "./stores/settingsStore";
import { useApplyTheme } from "./lib/hooks/useApplyTheme";
import { initDeepLinks } from "./lib/deepLinks";

import { BackgroundLayers } from "./components/effects/BackgroundLayers";
import { Scene3D } from "./components/effects/Scene3D";
import { CustomCursor } from "./components/effects/CustomCursor";
import { WideAmbient } from "./components/effects/WideAmbient";
import { Header } from "./components/Header";
import { PowerStack } from "./components/PowerStack";
import { Welcome } from "./components/Welcome";
import { ServerSelector } from "./components/ServerSelector";
import { ModeSegment } from "./components/ModeSegment";
import { Footer } from "./components/Footer";
import { SettingsPage } from "./components/SettingsPage";
import { openDashboard } from "./lib/openExternal";

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
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  const loadCached = useSubscriptionStore((s) => s.loadCached);
  const loadDeviceHwid = useSubscriptionStore((s) => s.loadDeviceHwid);
  const pingAll = useSubscriptionStore((s) => s.pingAll);

  // Settings
  const refreshOnOpen = useSettingsStore((x) => x.refreshOnOpen);
  const pingOnOpen = useSettingsStore((x) => x.pingOnOpen);
  const connectOnOpenSetting = useSettingsStore((x) => x.connectOnOpen);
  const autoRefresh = useSettingsStore((x) => x.autoRefresh);
  const autoRefreshHours = useSettingsStore((x) => x.autoRefreshHours);

  const [settingsOpen, setSettingsOpen] = useState(false);

  // Применяем активную тему (data-theme на <html>). См. App.css :root[data-theme="light"].
  useApplyTheme();

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
              {servers.length === 0 ? <Welcome /> : <ServerSelector />}
            </div>
            {errorMessage && (
              <pre className="hero-error grid-error">{errorMessage}</pre>
            )}
            <div className="grid-mode">
              <ModeSegment
                mode={mode}
                onChange={setMode}
                disabled={isRunning || isBusy}
              />
            </div>
          </div>

          {/* Быстрый доступ в личный кабинет с главного экрана.
              Скрываем когда показан Welcome — там уже есть своя кнопка,
              чтобы не было дублирования и UI помещался без скролла. */}
          {servers.length > 0 && (
            <button
              type="button"
              onClick={openDashboard}
              className="dashboard-link"
            >
              <span>личный кабинет</span>
              <span className="dashboard-link-arrow">→</span>
            </button>
          )}

          <Footer />
        </div>
      </div>

      {settingsOpen && (
        <SettingsPage onClose={() => setSettingsOpen(false)} />
      )}
    </>
  );
}

export default App;
