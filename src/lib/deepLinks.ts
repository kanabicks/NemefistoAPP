import { onOpenUrl, getCurrent } from "@tauri-apps/plugin-deep-link";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useVpnStore } from "../stores/vpnStore";
import { showToast } from "../stores/toastStore";

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
    case "routing":
    case "autorouting": {
      // 11.D расширенные deep-links для routing-профилей. Формат:
      //   nemefisto://routing/{add|onadd}/{base64|url}
      //   nemefisto://autorouting/{add|onadd}/{url}
      // segments тут — [verb, ...payload]. queryUrl/queryData как
      // альтернативные источники payload (для длинных base64).
      const verb = (segments.shift() || "").toLowerCase();
      if (verb !== "add" && verb !== "onadd") {
        console.warn("[deep-link] routing: неизвестный verb:", verb);
        return;
      }
      const raw = segments.join("/") || queryData || queryUrl || "";
      const decodedRaw = decodeUriOrPassthrough(raw);
      handleRoutingDeepLink(action, verb, decodedRaw);
      void focusMainWindow();
      break;
    }
    default:
      console.warn("[deep-link] неизвестное действие:", action);
  }
}

function decodeUriOrPassthrough(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/**
 * 11.D — обработчик routing/autorouting deep-links.
 *
 * - `routing/add/{base64-or-url}` — добавить статический профиль
 *   (без активации). Если payload — URL, скачиваем JSON один раз и
 *   сохраняем как Static.
 * - `routing/onadd/{base64-or-url}` — то же + сразу активируем.
 * - `autorouting/add/{url}` — добавить URL-источник с авто-обновлением
 *   (default 24ч). Без активации.
 * - `autorouting/onadd/{url}` — то же + активируем.
 */
async function handleRoutingDeepLink(
  kind: "routing" | "autorouting",
  verb: "add" | "onadd",
  raw: string
) {
  if (!raw) {
    showToast({
      kind: "warning",
      title: "routing deep-link",
      message: "пустой payload",
    });
    return;
  }

  try {
    let id: string;
    if (kind === "autorouting") {
      // payload должен быть URL
      if (!/^https?:\/\//i.test(raw)) {
        showToast({
          kind: "error",
          title: "autorouting",
          message: "ожидался URL, получено:\n" + raw.slice(0, 80),
        });
        return;
      }
      id = await invoke<string>("routing_add_url", {
        url: raw,
        intervalHours: 24,
      });
    } else {
      // routing: или base64-encoded JSON, или URL (тогда скачиваем
      // одноразово через routing_add_url + interval=8760 = «раз в год»
      // ≈ no-update; либо лучше: качаем один раз сами и кидаем в
      // routing_add_static как JSON).
      if (/^https?:\/\//i.test(raw)) {
        // Одноразовое скачивание — используем routing_add_url с
        // эффективным «no-update» интервалом (8760ч = 1 год).
        id = await invoke<string>("routing_add_url", {
          url: raw,
          intervalHours: 8760,
        });
      } else {
        // base64 / JSON
        id = await invoke<string>("routing_add_static", { payload: raw });
      }
    }

    if (verb === "onadd") {
      await invoke("routing_set_active", { id });
    }
    showToast({
      kind: "success",
      title: "routing-профиль",
      message:
        verb === "onadd"
          ? "добавлен и активирован"
          : "добавлен (активируйте в Settings)",
    });
  } catch (e) {
    showToast({
      kind: "error",
      title: "routing deep-link",
      message: String(e),
    });
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
