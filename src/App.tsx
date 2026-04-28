import { useEffect, useState } from "react";
import "./App.css";
import { useVpnStore } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";
import { useSettingsStore } from "./stores/settingsStore";
import { initDeepLinks } from "./lib/deepLinks";

import { BackgroundLayers } from "./components/effects/BackgroundLayers";
import { Header } from "./components/Header";
import { PowerStack } from "./components/PowerStack";
import { Welcome } from "./components/Welcome";
import { ServerSelector } from "./components/ServerSelector";
import { ModeSegment } from "./components/ModeSegment";
import { Footer } from "./components/Footer";
import { SettingsPage } from "./components/SettingsPage";

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

      <div className="app">
        <div className="frame">
          <Header onOpenSettings={() => setSettingsOpen(true)} />

          <PowerStack canConnect={canConnect} />

          {servers.length === 0 ? <Welcome /> : <ServerSelector />}

          {errorMessage && (
            <pre className="hero-error" style={{ marginTop: 12 }}>
              {errorMessage}
            </pre>
          )}

          <ModeSegment
            mode={mode}
            onChange={setMode}
            disabled={isRunning || isBusy}
          />

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
