import { useEffect, useRef, useState } from "react";
import { useVpnStore } from "../../stores/vpnStore";
import { useSubscriptionStore } from "../../stores/subscriptionStore";
import { useUtcClock } from "../../lib/hooks/useUtcClock";
import { useMediaQuery } from "../../lib/hooks/useMediaQuery";
import { APP_VERSION } from "../../lib/constants";

/**
 * Декоративные «ambient» текстовые блоки по бокам экрана для широких окон.
 * Заполняют пустоту слева/справа от mobile-формата центрального UI
 * полезной мета-информацией: статус системы, текущий узел/пинг, версия,
 * uptime сессии. Появляются только на ≥1280px — на узких display: none.
 *
 * Не интерактивны (pointer-events: none), не перехватывают клики.
 */
export function WideAmbient() {
  const isWide = useMediaQuery("(min-width: 1280px)");
  const status = useVpnStore((s) => s.status);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const socksPort = useVpnStore((s) => s.socksPort);
  const servers = useSubscriptionStore((s) => s.servers);
  const pings = useSubscriptionStore((s) => s.pings);
  const utcTime = useUtcClock();

  const selectedServer =
    selectedIndex !== null ? servers[selectedIndex] : null;
  const selectedPing =
    selectedIndex !== null ? pings[selectedIndex] : null;

  // Uptime сессии (с момента mount приложения) — мелкая ambient-метрика.
  const uptimeStart = useRef(Date.now());
  const [uptime, setUptime] = useState("00:00:00");
  useEffect(() => {
    if (!isWide) return;
    const tick = () => {
      const ms = Date.now() - uptimeStart.current;
      const s = Math.floor(ms / 1000) % 60;
      const m = Math.floor(ms / 60000) % 60;
      const h = Math.floor(ms / 3600000);
      const pad = (n: number) => String(n).padStart(2, "0");
      setUptime(`${pad(h)}:${pad(m)}:${pad(s)}`);
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [isWide]);

  if (!isWide) return null;

  const statusLabel: Record<typeof status, string> = {
    stopped: "STANDBY",
    starting: "CONNECTING",
    running: "ONLINE",
    stopping: "DISCONNECTING",
    error: "ERROR",
  };

  return (
    <>
      {/* Левая колонка — состояние системы */}
      <aside className="wide-side wide-side-left">
        <div className="wide-side-block">
          <div className="wide-side-key">/ status</div>
          <div className="wide-side-val">{statusLabel[status]}</div>
        </div>
        <div className="wide-side-block">
          <div className="wide-side-key">/ utc</div>
          <div className="wide-side-val wide-side-val-mono">{utcTime}</div>
        </div>
        <div className="wide-side-block">
          <div className="wide-side-key">/ session uptime</div>
          <div className="wide-side-val wide-side-val-mono">{uptime}</div>
        </div>
        <div className="wide-side-block">
          <div className="wide-side-key">/ build</div>
          <div className="wide-side-val">v.{APP_VERSION}</div>
          <div className="wide-side-sub">xray-core 26 · 2026.4</div>
        </div>
      </aside>

      {/* Правая колонка — данные подключения */}
      <aside className="wide-side wide-side-right">
        <div className="wide-side-block">
          <div className="wide-side-key">/ node</div>
          <div className="wide-side-val">
            {selectedServer ? selectedServer.name : "—"}
          </div>
          {selectedPing != null && (
            <div className="wide-side-sub">~ {selectedPing} ms rtt</div>
          )}
        </div>
        <div className="wide-side-block">
          <div className="wide-side-key">/ nodes total</div>
          <div className="wide-side-val">{servers.length || "—"}</div>
        </div>
        {socksPort && (
          <div className="wide-side-block">
            <div className="wide-side-key">/ inbound</div>
            <div className="wide-side-val wide-side-val-mono">
              socks5 :{socksPort}
            </div>
          </div>
        )}
        <div className="wide-side-block">
          <div className="wide-side-key">/ stack</div>
          <div className="wide-side-val">vless · reality</div>
          <div className="wide-side-sub">no-logs · no-retention</div>
        </div>
      </aside>
    </>
  );
}
