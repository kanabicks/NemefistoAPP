/**
 * Обёртка над `tauri-plugin-notification` для нативных Windows toast'ов
 * (Action Center).
 *
 * Зачем:
 * - In-app toaster показывается только в видимом окне; если юзер свернул
 *   приложение или закрыл в трей — он пропускает важные события (connect,
 *   kill-switch trigger, update). Нативный toast виден всегда.
 *
 * Принципы:
 *  1. **Native ИЛИ in-app, не оба.** `shouldUseNative()` решает по
 *     visibility окна. visible → in-app; hidden → native.
 *  2. **Permission лениво.** Не дёргаем `requestPermission()` на старте app
 *     (юзер пугается «зачем VPN-у мои уведомления»). Запрашиваем при
 *     первом реальном событии.
 *  3. **Debounce.** Один тост одного типа в 30-секундное окно. Ловим
 *     дребезг при network-flip / failover.
 *  4. **Settings toggle.** Юзер может отключить нативные toast'ы целиком —
 *     останется только in-app (`settingsStore.nativeNotifications`).
 *  5. **Graceful failure.** Permission denied / Win10 N edition без
 *     Notification API / любая ошибка плагина → no-op. Не падаем.
 */

import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSettingsStore } from "../stores/settingsStore";

/** Категория события — для debounce-ключа и (на будущее) фильтрации. */
export type NotifyKind =
  | "connected"
  | "disconnected"
  | "update-available"
  | "kill-switch-trigger"
  | "leak-detected";

/** Аргумент `notifyNative`. */
export interface NotifyArgs {
  kind: NotifyKind;
  title: string;
  body: string;
  /**
   * Если true — игнорируем visibility-проверку и шлём нативный toast
   * даже когда окно visible. Используем только для критических событий
   * типа kill-switch trigger, где юзер должен узнать СРАЗУ.
   */
  forceNative?: boolean;
}

/** Результат `requestPermission()` cache'ится — одна попытка на жизнь сессии. */
let permissionState: "unknown" | "granted" | "denied" | "asking" = "unknown";

/** Debounce-карта: kind → последний timestamp отправки (мс). */
const lastSent = new Map<NotifyKind, number>();
const DEBOUNCE_MS = 30_000;

/**
 * Проверить нужно ли использовать нативный toast вместо in-app.
 *
 * `true` если:
 *  - окно скрыто/свёрнуто (`visible === false` или `minimized`);
 *  - либо `forceNative === true`.
 *
 * При visible+focused окне — `false` (in-app toaster справится).
 *
 * Вызов через try/catch — getCurrentWindow() может throw'ить если контекст
 * вне Tauri (теоретически в Vitest), не падаем.
 */
export async function shouldUseNative(forceNative = false): Promise<boolean> {
  if (forceNative) return true;
  try {
    const win = getCurrentWindow();
    const [visible, minimized] = await Promise.all([
      win.isVisible().catch(() => true),
      win.isMinimized().catch(() => false),
    ]);
    return !visible || minimized;
  } catch {
    return false;
  }
}

/**
 * Лениво получить permission. Кешируется в `permissionState` —
 * `requestPermission()` дёргается один раз за сессию (либо до granted,
 * либо до denied).
 */
async function ensurePermission(): Promise<boolean> {
  if (permissionState === "granted") return true;
  if (permissionState === "denied") return false;
  if (permissionState === "asking") return false; // race-protection

  permissionState = "asking";
  try {
    if (await isPermissionGranted()) {
      permissionState = "granted";
      return true;
    }
    const result = await requestPermission();
    permissionState = result === "granted" ? "granted" : "denied";
    return permissionState === "granted";
  } catch {
    // Win10 N / API недоступен / прочее — graceful fallback.
    permissionState = "denied";
    return false;
  }
}

/**
 * Отправить нативный toast.
 *
 * No-op если:
 *  - settingsStore.nativeNotifications === false;
 *  - окно visible (если не forceNative);
 *  - этот kind уже слал toast в последние 30 сек;
 *  - permission denied.
 */
export async function notifyNative(args: NotifyArgs): Promise<void> {
  // 1. Глобальный switch.
  const enabled = useSettingsStore.getState().nativeNotifications;
  if (!enabled) return;

  // 2. Native vs in-app.
  if (!(await shouldUseNative(args.forceNative))) return;

  // 3. Debounce.
  const now = Date.now();
  const last = lastSent.get(args.kind);
  if (last !== undefined && now - last < DEBOUNCE_MS) return;

  // 4. Permission.
  if (!(await ensurePermission())) return;

  // 5. Send.
  try {
    sendNotification({ title: args.title, body: args.body });
    lastSent.set(args.kind, now);
  } catch {
    // Любая ошибка плагина (Win10 N / corrupted manifest / etc.) →
    // тихо игнорируем. In-app toaster уже мог показать сообщение
    // если окно было visible.
  }
}

/**
 * Сбросить debounce-кеш для конкретного типа. Используется когда мы хотим
 * принудительно показать toast независимо от того что недавно был такой же
 * (например юзер вручную вызвал тест уведомлений из Settings).
 */
export function resetNotifyDebounce(kind?: NotifyKind): void {
  if (kind) {
    lastSent.delete(kind);
  } else {
    lastSent.clear();
  }
}
