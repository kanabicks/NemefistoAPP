import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getAllWindows } from "@tauri-apps/api/window";
import { useVpnStore } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";
import "./FloatingApp.css";

/**
 * Плавающее мини-окно (этап 13.O). Запускается во втором Tauri-окне
 * с label `"floating"`, рендерится из того же entrypoint что и
 * главное приложение (см. `main.tsx`).
 *
 * Содержимое:
 * - **status-dot** — клик toggle'ит VPN (connect/disconnect);
 * - **имя сервера** — truncate если длинное;
 * - **скорость** — ↑ uplink / ↓ downlink в KB/s или MB/s,
 *   обновляется раз в секунду через `bandwidth-tick` event.
 *
 * Двойной клик в любую область — открывает главное окно.
 *
 * Окно полупрозрачное, alwaysOnTop, decorationless. Перетаскивается
 * за корневой div через `data-tauri-drag-region`.
 */
export function FloatingApp() {
  const status = useVpnStore((s) => s.status);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const refresh = useVpnStore((s) => s.refresh);
  const connect = useVpnStore((s) => s.connect);
  const disconnect = useVpnStore((s) => s.disconnect);
  const servers = useSubscriptionStore((s) => s.servers);
  const loadCached = useSubscriptionStore((s) => s.loadCached);

  const [bw, setBw] = useState<{ up: number; down: number }>({ up: 0, down: 0 });

  // Скоуп CSS: ставим класс на <html> чтобы правила из FloatingApp.css
  // (`html.is-floating { background: transparent }` и т.д.) применялись
  // только в этом окне. Vite бандлит CSS обоих окон в один файл, без
  // скоупа правила убивали бы темы главного окна.
  useEffect(() => {
    document.documentElement.classList.add("is-floating");
    return () => {
      document.documentElement.classList.remove("is-floating");
    };
  }, []);

  // Главное окно само поднимает stores из IPC и кеша; floating-окно
  // живёт в отдельном webview-процессе, у него свои Zustand-сторы —
  // приходится ещё раз поднять кеш списка серверов и refresh статуса
  // VPN, иначе внутри floating всё видится как «нет сервера, stopped».
  useEffect(() => {
    void loadCached();
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{ up_bps: number; down_bps: number }>(
      "bandwidth-tick",
      (event) => {
        setBw({ up: event.payload.up_bps, down: event.payload.down_bps });
      }
    ).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  // Слушаем смены VPN-статуса из главного окна. Tauri broadcast'ит
  // emit-ы во все окна, но vpnStore состояние локально per-window —
  // здесь мы просто refresh'имся при любых служебных событиях.
  useEffect(() => {
    let unlistens: Array<() => void> = [];
    const onAny = () => void refresh();
    Promise.all([
      listen("vpn-status-changed", onAny),
      listen("tray-action", onAny),
    ]).then((fns) => {
      unlistens = fns;
    });
    return () => {
      unlistens.forEach((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const isRunning = status === "running";
  const isBusy = status === "starting" || status === "stopping";
  const isError = status === "error";
  const selectedName =
    selectedIndex !== null && servers[selectedIndex]
      ? servers[selectedIndex].name
      : null;

  const onDotClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isBusy) return;
    if (isRunning) {
      void disconnect();
    } else if (selectedIndex !== null) {
      void connect();
    }
  };

  const onDoubleClick = async () => {
    try {
      const wins = await getAllWindows();
      const main = wins.find((w) => w.label === "main");
      if (main) {
        await main.show();
        await main.unminimize();
        await main.setFocus();
      }
    } catch {
      // ignore
    }
  };

  const dotClass = isError
    ? "is-error"
    : isBusy
    ? "is-busy"
    : isRunning
    ? "is-running"
    : "";

  return (
    <div
      className="floating-shell"
      data-tauri-drag-region
      onDoubleClick={onDoubleClick}
    >
      <button
        type="button"
        className={`floating-dot ${dotClass}`}
        onClick={onDotClick}
        title={
          isRunning
            ? "vpn включён — клик чтобы отключить"
            : selectedIndex !== null
            ? "vpn выключен — клик чтобы подключить"
            : "выбери сервер в главном окне"
        }
      />
      <div className="floating-name" data-tauri-drag-region>
        {selectedName ?? "нет сервера"}
      </div>
      <div className="floating-bw" data-tauri-drag-region>
        <span>↑ {formatRate(bw.up)}</span>
        <span>↓ {formatRate(bw.down)}</span>
      </div>
    </div>
  );
}

/**
 * Форматирует bytes/sec в читаемый вид:
 *   - <1 KB/s — `0 B/s` (нулевая активность, не захламляем);
 *   - <1 MB/s — `123 KB/s` (целое число);
 *   - >=1 MB/s — `4.2 MB/s` (одна цифра после запятой).
 */
function formatRate(bps: number): string {
  if (bps < 1024) return "0 B/s";
  if (bps < 1024 * 1024) {
    return `${Math.round(bps / 1024)} KB/s`;
  }
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}
