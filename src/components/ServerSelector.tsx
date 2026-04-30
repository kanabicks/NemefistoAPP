import { useEffect, useMemo, useRef, useState } from "react";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useSettingsStore } from "../stores/settingsStore";
import { PingBadge } from "./PingBadge";

/**
 * Маленький бейдж рядом с пингом — показывает движок-совместимость
 * сервера (этап 8.B). Скрывается если совместимы оба ядра (общий случай) —
 * чтобы не захламлять список. Видим только для эксклюзивных протоколов:
 * TUIC/AnyTLS/Mieru → "M" (mihomo only), готовый Xray JSON → "X" (xray only).
 */
function EngineBadge({ compat }: { compat?: string[] }) {
  if (!compat || compat.length === 0 || compat.length > 1) return null;
  const e = compat[0];
  if (e !== "xray" && e !== "mihomo") return null;
  const label = e === "xray" ? "X" : "M";
  const title =
    e === "xray"
      ? "поддерживается только Xray"
      : "поддерживается только Mihomo";
  return (
    <span className="engine-badge" title={title} data-engine={e}>
      {label}
    </span>
  );
}

/**
 * Список серверов из подписки + текущий выбранный показан как pill.
 *
 * Pill кликабелен → разворачивается drawer со списком всех серверов.
 * Изменять выбор можно только пока туннель отключён — иначе перепрыжки
 * в середине сессии (с обрывом текущего соединения).
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
  // Delayed unmount: после клика «закрыть» rows должны отыграть leave-
  // анимацию, прежде чем DOM удалится. drawerMounted держит элементы
  // дополнительно ~CLOSE_DURATION_MS пока drawerOpen=false.
  const [drawerMounted, setDrawerMounted] = useState(false);
  const isClosing = drawerMounted && !drawerOpen;
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (drawerOpen) {
      setDrawerMounted(true);
      // Принудительно сбрасываем scroll в 0 на следующий tick — иначе
      // browser scroll-anchoring мог уже прокрутить вниз чтобы
      // «удержаться» за анимирующийся первый row, и пользователь
      // не увидит его.
      requestAnimationFrame(() => {
        if (listRef.current) listRef.current.scrollTop = 0;
      });
    } else if (drawerMounted) {
      const t = setTimeout(() => setDrawerMounted(false), 600);
      return () => clearTimeout(t);
    }
  }, [drawerOpen, drawerMounted]);

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

      {/* Drawer всегда в DOM — переключение через CSS-класс is-open даёт
          плавный grid-rows transition (0fr → 1fr). Содержимое монтируется
          только когда открыт — каскадная stagger-animation запускается
          на каждое раскрытие, при закрытии rows резко уходят, но скрыты
          overflow:hidden родителя. */}
      <div
        className={`server-drawer${drawerOpen ? " is-open" : ""}${
          isClosing ? " is-closing" : ""
        }`}
      >
        <div className="server-drawer-inner">
          {drawerMounted && (
            <>
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
              <div className="server-list" ref={listRef}>
                {sortedIndices.map((i, idx) => {
                  const s = servers[i];
                  return (
                    <div
                      key={i}
                      className={
                        "server-row" +
                        (selectedIndex === i ? " is-selected" : "") +
                        (isRunning ? " is-disabled" : "")
                      }
                      // --i задаёт animation-delay через CSS — каскадное появление.
                      style={{ ["--i" as string]: idx } as React.CSSProperties}
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
                      <EngineBadge compat={s.engine_compat} />
                      <PingBadge ms={pings[i]} loading={pingsLoading} />
                      {selectedIndex === i && (
                        <span className="server-row-check">✓</span>
                      )}
                    </div>
                  );
                })}
              </div>
            </>
          )}
        </div>
      </div>
    </>
  );
}
