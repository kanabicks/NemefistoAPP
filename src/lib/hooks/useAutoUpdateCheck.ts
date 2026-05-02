/**
 * 14.A: периодическая проверка обновлений приложения.
 *
 * Стратегия:
 * 1. На mount (если `autoCheckUpdates: true`) — проверяем через 10 сек
 *    после старта (чтобы не задержать UI на холодном запуске).
 * 2. Каждые 6 часов — повторная проверка.
 * 3. Если обновление есть И версия НЕ в `dismissedUpdateVersions` —
 *    переводим updateStore в `available`, UI показывает modal.
 *
 * Cooldown через `lastCheckAt` гарантирует что повторный mount хука
 * (после ремоунта или dev-rebuild) не дёргает endpoint лишний раз.
 */

import { useEffect } from "react";
import { useSettingsStore } from "../../stores/settingsStore";
import { useUpdateStore } from "../../stores/updateStore";
import { checkForUpdates } from "../updater";

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000; // 6 часов
const STARTUP_DELAY_MS = 10_000;

export function useAutoUpdateCheck() {
  const autoCheck = useSettingsStore((s) => s.autoCheckUpdates);
  const dismissed = useSettingsStore((s) => s.dismissedUpdateVersions);
  const setUpdateState = useUpdateStore((s) => s.setState);
  const setLastCheckAt = useUpdateStore((s) => s.setLastCheckAt);

  useEffect(() => {
    if (!autoCheck) return;

    let cancelled = false;
    let intervalId: number | undefined;

    const runCheck = async () => {
      if (cancelled) return;
      const last = useUpdateStore.getState().lastCheckAt;
      if (Date.now() - last < CHECK_INTERVAL_MS / 2) return; // cooldown

      setUpdateState({ kind: "checking" });
      setLastCheckAt(Date.now());

      const update = await checkForUpdates();
      if (cancelled) return;

      if (!update) {
        setUpdateState({ kind: "idle" });
        return;
      }
      // Юзер уже dismiss'нул эту версию — не показываем повторно.
      const skipped = useSettingsStore.getState().dismissedUpdateVersions;
      if (skipped.includes(update.version)) {
        setUpdateState({ kind: "idle" });
        return;
      }
      setUpdateState({ kind: "available", update });
    };

    const startupTimer = window.setTimeout(runCheck, STARTUP_DELAY_MS);
    intervalId = window.setInterval(runCheck, CHECK_INTERVAL_MS);

    return () => {
      cancelled = true;
      window.clearTimeout(startupTimer);
      if (intervalId !== undefined) window.clearInterval(intervalId);
    };
    // dismissed не в deps — мы читаем актуальное значение через getState() внутри.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoCheck, setUpdateState, setLastCheckAt]);
  void dismissed;
}
