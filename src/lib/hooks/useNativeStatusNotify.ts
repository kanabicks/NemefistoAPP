/**
 * Хук-наблюдатель: ловит переходы статуса VPN и состояния updater'а,
 * шлёт нативные toast'ы когда главное окно невидимо.
 *
 * Особенности:
 *  - **Skip first transition**. При старте app `vpnStore.refresh()` ставит
 *    статус сразу в `running` если sing-box ещё запущен с прошлой
 *    сессии. Это не «пользователь подключился» — это «обнаружили что
 *    уже было подключено». Ложный toast недопустим. Решение: первый
 *    subscribe-callback игнорируется (`isFirstRef`).
 *  - **Только running ↔ stopped**. error/starting/stopping/прочие
 *    промежутки не шлют нативный toast. error уже обрабатывается через
 *    in-app showToast в vpnStore. Если хотим в будущем — добавим.
 *  - **Server name in body**. Для «Подключено» подтягиваем имя сервера
 *    из subscriptionStore через `selectedIndex`.
 *  - **Update available**. Слушаем updateStore. Только переход idle/checking →
 *    available шлёт toast (downloading/installed/error не шлют, они для
 *    UI-модалки).
 *
 * Регистрировать единожды в `App.tsx` (после refresh()).
 */

import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useVpnStore } from "../../stores/vpnStore";
import { useSubscriptionStore } from "../../stores/subscriptionStore";
import { useUpdateStore } from "../../stores/updateStore";
import { notifyNative } from "../nativeNotify";

export function useNativeStatusNotify(): void {
  const { t } = useTranslation();
  const isFirstStatusRef = useRef(true);
  const prevStatusRef = useRef<ReturnType<typeof useVpnStore.getState>["status"]>(
    useVpnStore.getState().status,
  );
  const prevUpdateKindRef = useRef<string>(useUpdateStore.getState().state.kind);

  // VPN status transitions.
  useEffect(() => {
    const unsub = useVpnStore.subscribe((state) => {
      const next = state.status;
      const prev = prevStatusRef.current;
      prevStatusRef.current = next;

      // Skip самый первый переход — это initial refresh().
      if (isFirstStatusRef.current) {
        isFirstStatusRef.current = false;
        return;
      }
      if (prev === next) return;

      if (prev !== "running" && next === "running") {
        // Connected. Достаём имя сервера для body.
        const sub = useSubscriptionStore.getState();
        const idx = useVpnStore.getState().selectedIndex;
        const server =
          idx !== null && sub.servers[idx]
            ? sub.servers[idx].name
            : t("notification.connected.unknownServer");
        void notifyNative({
          kind: "connected",
          title: t("notification.connected.title"),
          body: t("notification.connected.body", { server }),
        });
      } else if (prev === "running" && next === "stopped") {
        // Clean disconnect.
        void notifyNative({
          kind: "disconnected",
          title: t("notification.disconnected.title"),
          body: t("notification.disconnected.body"),
        });
      } else if (next === "error" && prev !== "error") {
        // Network failure / kill-switch / force disconnect — критично.
        // Шлём принудительно (forceNative), даже если окно visible —
        // юзер мог отвернуться, важное событие.
        void notifyNative({
          kind: "kill-switch-trigger",
          title: t("notification.error.title"),
          body: t("notification.error.body"),
          forceNative: true,
        });
      }
    });
    return unsub;
  }, [t]);

  // Update available transitions.
  useEffect(() => {
    const unsub = useUpdateStore.subscribe((state) => {
      const nextKind = state.state.kind;
      const prevKind = prevUpdateKindRef.current;
      prevUpdateKindRef.current = nextKind;

      if (prevKind !== "available" && nextKind === "available") {
        const update =
          state.state.kind === "available" ? state.state.update : null;
        if (!update) return;
        void notifyNative({
          kind: "update-available",
          title: t("notification.updateAvailable.title"),
          body: t("notification.updateAvailable.body", {
            version: update.version,
          }),
        });
      }
    });
    return unsub;
  }, [t]);
}
