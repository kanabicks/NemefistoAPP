import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { useVpnStore } from "../stores/vpnStore";

/**
 * Текущая скорость передачи данных (этап 13.O).
 *
 * Слушает `bandwidth-tick` — Rust-сервис в `platform/bandwidth.rs`
 * каждую секунду читает `GetIfTable2` для default-route интерфейса
 * и эмитит `{ up_bps, down_bps, iface }`.
 *
 * Показывается только когда VPN активен (`status === "running"`):
 * - в TUN-режиме default-route уходит в наш WinTUN — счётчик
 *   показывает именно VPN-трафик;
 * - в proxy-режиме default-route на физическом интерфейсе —
 *   счётчик показывает весь системный трафик (включая VPN).
 *
 * `iface` опционально показываем в title — чтобы при подозрении
 * «почему 0?» сразу видеть какой интерфейс мониторится.
 */
export function BandwidthMeter() {
  const { t } = useTranslation();
  const status = useVpnStore((s) => s.status);
  const [tick, setTick] = useState<{
    up: number;
    down: number;
    iface: string | null;
  }>({ up: 0, down: 0, iface: null });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{ up_bps: number; down_bps: number; iface: string | null }>(
      "bandwidth-tick",
      (event) => {
        setTick({
          up: event.payload.up_bps,
          down: event.payload.down_bps,
          iface: event.payload.iface,
        });
      }
    ).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, []);

  if (status !== "running") return null;

  return (
    <div
      className="bandwidth-meter"
      title={
        tick.iface
          ? t("bandwidth.titleIface", { iface: tick.iface })
          : t("bandwidth.titleNoIface")
      }
    >
      <span className="bw-arrow">↑</span>
      <span className="bw-value">{formatRate(tick.up)}</span>
      <span className="bw-arrow">↓</span>
      <span className="bw-value">{formatRate(tick.down)}</span>
    </div>
  );
}

function formatRate(bps: number): string {
  if (bps < 1024) return "0 B/s";
  if (bps < 1024 * 1024) return `${Math.round(bps / 1024)} KB/s`;
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}
