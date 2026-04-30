import { onOpenUrl, getCurrent } from "@tauri-apps/plugin-deep-link";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useVpnStore } from "../stores/vpnStore";

/**
 * Поддерживаемые deep-link-ссылки:
 *
 *  Управление VPN:
 *   nemefisto://connect | open                подключить выбранный сервер
 *   nemefisto://disconnect | close            отключиться
 *   nemefisto://toggle                        переключить состояние
 *   nemefisto://status                        вынести окно вперёд
 *
 *  Импорт подписки (поддерживается оба синтаксиса):
 *   nemefisto://add?url=<encoded-url>         query-форма
 *   nemefisto://add/<encoded-url-or-base64>   path-форма
 *   nemefisto://import/<...>                  alias
 *   nemefisto://onadd/<url>                   импорт + сразу подключение
 *   nemefisto://import?data=<base64>          альтернативный query-параметр
 *
 *  Auto-detect для path-формы import: если значение начинается с
 *  http(s):// — это URL подписки. Иначе пробуем base64-декод и
 *  проверяем на http(s):// внутри.
 */

/** Декодирует строку: сначала URL-decode, затем если не похоже на URL —
 *  пробуем base64. Возвращает первое валидное http(s):// или null. */
function detectSubscriptionUrl(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  // 1. Уже URL (после URL-decode)
  let candidate = trimmed;
  try {
    candidate = decodeURIComponent(trimmed);
  } catch {
    // не получилось декодировать — пробуем как есть
  }
  if (/^https?:\/\//i.test(candidate)) return candidate;
  // 2. base64 → URL
  try {
    const decoded = atob(candidate.replace(/-/g, "+").replace(/_/g, "/"));
    if (/^https?:\/\//i.test(decoded)) return decoded.trim();
  } catch {
    // не base64 — игнорируем
  }
  return null;
}

async function focusMainWindow() {
  try {
    const w = getCurrentWindow();
    await w.show();
    await w.unminimize();
    await w.setFocus();
  } catch (e) {
    console.warn("[deep-link] не удалось вынести окно вперёд:", e);
  }
}

export function handleDeepLink(rawUrl: string) {
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    console.warn("[deep-link] невалидный URL:", rawUrl);
    return;
  }

  if (parsed.protocol !== "nemefisto:") {
    console.warn("[deep-link] чужая схема:", parsed.protocol);
    return;
  }

  // host у nemefisto://action — пустой на одних платформах, заполнен на
  // других. Action всегда первый сегмент: либо host, либо первая часть
  // pathname. payload (если есть) — остальное pathname (для path-формы)
  // или ?url=/?data= параметры.
  const segments = parsed.pathname.split("/").filter((s) => s.length > 0);
  const action = (parsed.host || segments.shift() || "").toLowerCase();
  const pathPayload = segments.join("/"); // для import/onadd
  const queryUrl = parsed.searchParams.get("url");
  const queryData = parsed.searchParams.get("data");

  switch (action) {
    case "add":
    case "import":
    case "onadd": {
      const raw = pathPayload || queryUrl || queryData;
      if (!raw) {
        console.warn("[deep-link] import без payload");
        return;
      }
      const url = detectSubscriptionUrl(raw);
      if (!url) {
        console.warn("[deep-link] не удалось извлечь URL подписки из:", raw);
        return;
      }
      const sub = useSubscriptionStore.getState();
      sub.setUrl(url);
      void sub.fetchSubscription().then(() => {
        // onadd → сразу пробуем подключение если выбран сервер
        if (action === "onadd") {
          const vpn = useVpnStore.getState();
          if (vpn.status === "stopped" && vpn.selectedIndex !== null) {
            void vpn.connect();
          }
        }
      });
      void focusMainWindow();
      break;
    }
    case "connect":
    case "open": {
      const vpn = useVpnStore.getState();
      if (vpn.status === "stopped") void vpn.connect();
      void focusMainWindow();
      break;
    }
    case "disconnect":
    case "close": {
      const vpn = useVpnStore.getState();
      if (vpn.status === "running") void vpn.disconnect();
      break;
    }
    case "toggle": {
      const vpn = useVpnStore.getState();
      if (vpn.status === "running") void vpn.disconnect();
      else if (vpn.status === "stopped") void vpn.connect();
      void focusMainWindow();
      break;
    }
    case "status": {
      // Просто вынести приложение на передний план. Полезно для интеграций
      // (виджет/скрипт хочет «открой клиент»).
      void focusMainWindow();
      break;
    }
    default:
      console.warn("[deep-link] неизвестное действие:", action);
  }
}

/**
 * Регистрирует подписку на deep-link события и обрабатывает «холодный»
 * запуск (когда приложение запустили кликом по nemefisto://...).
 */
export async function initDeepLinks(): Promise<() => void> {
  // Cold start: процесс был запущен с deep-link-ом в args
  try {
    const initial = await getCurrent();
    if (initial && initial.length > 0) {
      for (const url of initial) handleDeepLink(url);
    }
  } catch {
    // на платформах без cold-start API getCurrent кидает — игнорируем
  }

  // Warm: пока приложение запущено, ОС вызывает onOpenUrl
  const unlisten = await onOpenUrl((urls) => {
    for (const url of urls) handleDeepLink(url);
  });
  return unlisten;
}
