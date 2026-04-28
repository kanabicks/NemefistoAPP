import { onOpenUrl, getCurrent } from "@tauri-apps/plugin-deep-link";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useVpnStore } from "../stores/vpnStore";

/**
 * Поддерживаемые deep-link-ссылки:
 *
 *  nemefisto://add?url=<encoded-subscription-url>     добавить подписку
 *  nemefisto://import?url=<encoded-subscription-url>  то же что add (alias)
 *  nemefisto://connect                                подключить выбранный сервер
 *  nemefisto://disconnect                             отключиться
 *  nemefisto://toggle                                 переключить состояние
 */
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

  // host у nemefisto://action — пустой, действие лежит в pathname или host
  // в зависимости от платформы. Поэтому пробуем оба.
  const action = (parsed.host || parsed.pathname.replace(/^\/+/, "")).toLowerCase();

  switch (action) {
    case "add":
    case "import": {
      const url = parsed.searchParams.get("url");
      if (!url) {
        console.warn("[deep-link] add/import без url");
        return;
      }
      const sub = useSubscriptionStore.getState();
      sub.setUrl(url);
      void sub.fetchSubscription();
      break;
    }
    case "connect": {
      const vpn = useVpnStore.getState();
      if (vpn.status === "stopped") void vpn.connect();
      break;
    }
    case "disconnect": {
      const vpn = useVpnStore.getState();
      if (vpn.status === "running") void vpn.disconnect();
      break;
    }
    case "toggle": {
      const vpn = useVpnStore.getState();
      if (vpn.status === "running") void vpn.disconnect();
      else if (vpn.status === "stopped") void vpn.connect();
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
