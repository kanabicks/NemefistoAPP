import { openUrl } from "@tauri-apps/plugin-opener";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { DASHBOARD_URL, SUPPORT_URL } from "./constants";

/** Открыть личный кабинет подписки.
 *  Если провайдер прислал `profile-web-page-url` — используется он;
 *  иначе — захардкоженный DASHBOARD_URL. */
export function openDashboard() {
  const meta = useSubscriptionStore.getState().meta;
  void openUrl(meta?.webPageUrl ?? DASHBOARD_URL);
}

/** Открыть страницу поддержки.
 *  Если провайдер прислал `support-url` — используется он;
 *  иначе — захардкоженный SUPPORT_URL. */
export function openSupport() {
  const meta = useSubscriptionStore.getState().meta;
  void openUrl(meta?.supportUrl ?? SUPPORT_URL);
}
