import { openUrl } from "@tauri-apps/plugin-opener";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { SUPPORT_URL } from "./constants";

/** Открыть личный кабинет подписки.
 *
 *  ВАЖНО (изменение поведения): URL берётся ТОЛЬКО из заголовка
 *  `profile-web-page-url`, который провайдер подписки прислал в HTTP-
 *  ответе. Если заголовка нет — функция ничего не делает (no-op).
 *
 *  Захардкоженный fallback (`web.nemefisto.online`) убран:
 *   - универсальный клиент не должен рекламировать конкретного
 *     провайдера;
 *   - для пользователей сторонних подписок ссылка на наш сайт не
 *     релевантна;
 *   - UI должен скрывать кнопку когда `useHasDashboardUrl() === false`. */
export function openDashboard() {
  const url = useSubscriptionStore.getState().meta?.webPageUrl;
  if (!url) return;
  void openUrl(url);
}

/** Hook для условного рендера кнопки «личный кабинет».
 *  Возвращает `true` только если подписка прислала
 *  `profile-web-page-url`. */
export function useHasDashboardUrl(): boolean {
  return !!useSubscriptionStore((s) => s.meta?.webPageUrl);
}

/** Открыть страницу поддержки.
 *  Если провайдер прислал `support-url` — используется он;
 *  иначе — захардкоженный SUPPORT_URL (общий бот). */
export function openSupport() {
  const meta = useSubscriptionStore.getState().meta;
  void openUrl(meta?.supportUrl ?? SUPPORT_URL);
}
