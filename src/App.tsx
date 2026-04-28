import { useEffect, useState } from "react";
import "./App.css";
import { useVpnStore, type VpnMode, type VpnStatus } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";

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
  tun: "tun",
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
  const subLoading = useSubscriptionStore((s) => s.loading);
  const subError = useSubscriptionStore((s) => s.error);
  const subUrl = useSubscriptionStore((s) => s.url);
  const subHwid = useSubscriptionStore((s) => s.hwid);
  const deviceHwid = useSubscriptionStore((s) => s.deviceHwid);
  const setSubUrl = useSubscriptionStore((s) => s.setUrl);
  const setSubHwid = useSubscriptionStore((s) => s.setHwid);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  const loadCached = useSubscriptionStore((s) => s.loadCached);
  const loadDeviceHwid = useSubscriptionStore((s) => s.loadDeviceHwid);

  // UI состояние раскрывашек
  const [serverDrawer, setServerDrawer] = useState(false);
  const [settingsDrawer, setSettingsDrawer] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [hwidCopied, setHwidCopied] = useState(false);
  const utcTime = useUtcClock();

  useEffect(() => {
    refresh();
    loadCached();
    loadDeviceHwid();
  }, [refresh, loadCached, loadDeviceHwid]);

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";
  const canConnect = selectedIndex !== null && !isBusy;
  const selectedServer =
    selectedIndex !== null ? servers[selectedIndex] : null;

  const onPowerClick = () => {
    if (isBusy) return;
    isRunning ? disconnect() : connect();
  };

  const copyHwid = async () => {
    if (!deviceHwid) return;
    try {
      await navigator.clipboard.writeText(deviceHwid);
      setHwidCopied(true);
      setTimeout(() => setHwidCopied(false), 1500);
    } catch {
      // буфер недоступен — без обратной связи
    }
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
            <div className="header-meta">
              <span className="blink">●</span>
              <span>{utcTime}</span>
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

          {/* ── Server pill (текущий выбранный) ─────────────────────────── */}
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
                <span className="server-pill-arrow">
                  {serverDrawer ? "▾" : "▸"}
                </span>
              </>
            ) : (
              <>
                <span className="server-pill-empty">
                  {servers.length > 0
                    ? "выбери сервер"
                    : "загрузи подписку ниже"}
                </span>
                <span className="server-pill-arrow">
                  {serverDrawer ? "▾" : "▸"}
                </span>
              </>
            )}
          </button>

          {/* Server list — выпадает по клику на pill */}
          {serverDrawer && servers.length > 0 && (
            <div className="server-list" style={{ marginTop: 8 }}>
              {servers.map((s, i) => (
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
                  {selectedIndex === i && (
                    <span className="server-row-check">✓</span>
                  )}
                </div>
              ))}
            </div>
          )}

          {errorMessage && (
            <pre className="hero-error" style={{ marginTop: 12 }}>
              {errorMessage}
            </pre>
          )}

          {/* ── Settings drawer ──────────────────────────────────────────── */}
          <div className="drawer">
            <button
              type="button"
              onClick={() => setSettingsDrawer((v) => !v)}
              className={`drawer-toggle${settingsDrawer ? " is-open" : ""}`}
            >
              <span>
                <span className="num">/ 01 — </span>настройки
              </span>
              <span className="arrow">▸</span>
            </button>
            {settingsDrawer && (
              <div className="drawer-body">
                {/* Подписка */}
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  <span className="field-label">подписка</span>
                  <div className="row-input">
                    <input
                      type="url"
                      value={subUrl}
                      onChange={(e) => setSubUrl(e.target.value)}
                      onKeyDown={(e) =>
                        e.key === "Enter" && fetchSubscription()
                      }
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
                  {subError && (
                    <pre className="hero-error">{subError}</pre>
                  )}
                </div>

                {/* HWID */}
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  <span className="field-label">hwid устройства</span>
                  <div className="hwid-row">
                    <span
                      className={"hwid-value" + (deviceHwid ? "" : " hwid-empty")}
                    >
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
                    отправь в бот для добавления устройства
                  </p>
                </div>

                {/* Режим */}
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  <span className="field-label">режим</span>
                  <div className="mode-seg">
                    {(["proxy", "tun"] as VpnMode[]).map((m) => (
                      <button
                        key={m}
                        type="button"
                        disabled={isRunning || isBusy}
                        onClick={() => setMode(m)}
                        className={mode === m ? "is-active" : ""}
                      >
                        {m === "proxy" ? "proxy" : "tun"}
                      </button>
                    ))}
                  </div>
                </div>

                {/* Advanced */}
                <button
                  type="button"
                  onClick={() => setAdvancedOpen((v) => !v)}
                  className="drawer-toggle"
                  style={{ padding: "8px 0", borderTop: "1px solid var(--line)" }}
                >
                  <span>
                    <span className="num">▸ </span>override hwid
                  </span>
                </button>
                {advancedOpen && (
                  <input
                    type="text"
                    value={subHwid}
                    onChange={(e) => setSubHwid(e.target.value)}
                    placeholder={
                      deviceHwid ||
                      "оставь пустым чтобы использовать системный hwid"
                    }
                    className="input"
                  />
                )}
              </div>
            )}
          </div>

          {/* ── Footer ──────────────────────────────────────────────────── */}
          <footer className="footer">
            <span>NO-LOGS · XRAY · VLESS · REALITY</span>
            <span>© 2026 NEMEFISTO · v.0.1.0</span>
          </footer>
        </div>
      </div>
    </>
  );
}

export default App;
