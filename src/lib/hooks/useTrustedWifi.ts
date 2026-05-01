import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useSettingsStore } from "../../stores/settingsStore";
import { useRuntimeStore } from "../../stores/runtimeStore";
import { useVpnStore } from "../../stores/vpnStore";

type WifiChange = { from: string | null; to: string | null };

/**
 * Реагирует на смену Wi-Fi сети (этап 13.M).
 *
 * Логика:
 * 1. **Попали в trusted SSID** + `trustedSsidAction === "disconnect"`
 *    → если VPN активен, отключаем его и помечаем
 *    `autoDisconnectedBySsid = true`. При следующем выходе из этой
 *    сети мы знаем что VPN выключали мы.
 * 2. **Ушли с trusted SSID** + `autoConnectOnLeave` + был
 *    `autoDisconnectedBySsid` → подключаемся обратно. Снимаем флаг.
 * 3. **Пользователь сам выключил VPN** в trusted-сети → флаг не
 *    ставится, при выходе ничего не происходит.
 *
 * Вызывается **только из главного окна** — listener привязан к
 * процессу, во floating-окне продублирует обработчик.
 */
export function useTrustedWifi() {
  const setCurrentSsid = useRuntimeStore((s) => s.setCurrentSsid);
  const setAutoFlag = useRuntimeStore((s) => s.setAutoDisconnectedBySsid);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void listen<WifiChange>("wifi-changed", (event) => {
      const { to } = event.payload;
      setCurrentSsid(to);

      const settings = useSettingsStore.getState();
      const runtime = useRuntimeStore.getState();
      const vpn = useVpnStore.getState();

      const enteredTrusted = to !== null && settings.trustedSsids.includes(to);
      const wasInTrusted =
        runtime.currentSsid !== null &&
        settings.trustedSsids.includes(runtime.currentSsid);

      if (enteredTrusted && settings.trustedSsidAction === "disconnect") {
        if (vpn.status === "running") {
          console.log("[trusted-wifi] зашли в", to, "— отключаем vpn");
          setAutoFlag(true);
          void vpn.disconnect();
        }
        return;
      }

      // Покинули доверенную сеть — переподключаем если мы сами выключали.
      if (
        wasInTrusted &&
        !enteredTrusted &&
        settings.autoConnectOnLeave &&
        runtime.autoDisconnectedBySsid &&
        vpn.selectedIndex !== null &&
        (vpn.status === "stopped" || vpn.status === "error")
      ) {
        console.log("[trusted-wifi] вышли из доверенной сети — reconnect");
        setAutoFlag(false);
        void vpn.connect();
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
