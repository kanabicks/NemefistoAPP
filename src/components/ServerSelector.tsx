import { useMemo, useState } from "react";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useSettingsStore } from "../stores/settingsStore";
import { PingBadge } from "./PingBadge";

/**
 * Список серверов из подписки + текущий выбранный показан как pill.
 * Клик по pill раскрывает выбор. Изменять выбор можно только пока
 * туннель отключён (исключаем перепрыжки в середине сессии).
 */
export function ServerSelector() {
  const status = useVpnStore((s) => s.status);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const selectServer = useVpnStore((s) => s.selectServer);

  const servers = useSubscriptionStore((s) => s.servers);
  const pings = useSubscriptionStore((s) => s.pings);
  const pingsLoading = useSubscriptionStore((s) => s.pingsLoading);
  const pingAll = useSubscriptionStore((s) => s.pingAll);
  const sort = useSettingsStore((s) => s.sort);

  const [drawerOpen, setDrawerOpen] = useState(false);

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";
  const selectedServer =
    selectedIndex !== null ? servers[selectedIndex] : null;

  // Сортировка серверов согласно настройкам
  const sortedIndices = useMemo(() => {
    const idx = servers.map((_, i) => i);
    if (sort === "none") return idx;
    if (sort === "name") {
      return idx.sort((a, b) =>
        servers[a].name.localeCompare(servers[b].name, "ru")
      );
    }
    if (sort === "ping") {
      return idx.sort((a, b) => {
        const pa = pings[a];
        const pb = pings[b];
        if (pa == null && pb == null) return 0;
        if (pa == null) return 1;
        if (pb == null) return -1;
        return pa - pb;
      });
    }
    return idx;
  }, [servers, sort, pings]);

  if (servers.length === 0) return null;

  return (
    <>
      <button
        type="button"
        className="server-pill"
        disabled={isRunning || isBusy}
        onClick={() => setDrawerOpen((v) => !v)}
      >
        {selectedServer ? (
          <>
            <span className="server-pill-num">
              / {String(selectedIndex! + 1).padStart(2, "0")}
            </span>
            <span className="server-pill-name">{selectedServer.name}</span>
            <PingBadge ms={pings[selectedIndex!]} loading={pingsLoading} />
            <span className="server-pill-arrow">
              {drawerOpen ? "▾" : "▸"}
            </span>
          </>
        ) : (
          <>
            <span className="server-pill-empty">выбери сервер</span>
            <span className="server-pill-arrow">▸</span>
          </>
        )}
      </button>

      {drawerOpen && (
        <div style={{ marginTop: 8 }}>
          <div className="server-list-head">
            <span>{servers.length} nodes</span>
            <button
              type="button"
              onClick={() => pingAll()}
              disabled={pingsLoading}
              className={`ping-refresh${pingsLoading ? " is-loading" : ""}`}
              title="обновить пинги"
              aria-label="обновить пинги"
            >
              ↻
            </button>
          </div>
          <div className="server-list">
            {sortedIndices.map((i) => {
              const s = servers[i];
              return (
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
                    setDrawerOpen(false);
                  }}
                >
                  <span className="server-row-num">
                    {String(i + 1).padStart(2, "0")}
                  </span>
                  <span className="server-row-name">{s.name}</span>
                  <PingBadge ms={pings[i]} loading={pingsLoading} />
                  {selectedIndex === i && (
                    <span className="server-row-check">✓</span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </>
  );
}
