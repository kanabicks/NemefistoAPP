import { useTranslation } from "react-i18next";
import { useUtcClock } from "../lib/hooks/useUtcClock";
import { openDashboard, useHasDashboardUrl } from "../lib/openExternal";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { SettingsIcon, UserIcon } from "./icons";

/**
 * Шапка приложения: лого + UTC-часы + + (добавить подписку) +
 * кнопки личного кабинета и настроек.
 */
export function Header({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { t } = useTranslation();
  const utcTime = useUtcClock();
  const hasDashboardUrl = useHasDashboardUrl();
  // 0.3.0: + кнопка для добавления подписки. Видна только когда уже есть
  // хотя бы одна (subscriptions[].length > 0). При первом запуске (Welcome)
  // прятать — там и так центральная форма ввода URL.
  const hasAnySubscription = useSubscriptionStore(
    (s) => s.subscriptions.length > 0 || s.url.trim() !== ""
  );

  return (
    <header className="header">
      <div className="header-logo">
        <img src="/logo.png" alt="" />
        <span>nemefisto</span>
      </div>
      <div className="header-right">
        <div className="header-meta">
          <span className="blink">●</span>
          <span>{utcTime}</span>
        </div>
        {hasAnySubscription && (
          <button
            type="button"
            className="icon-btn"
            onClick={() => {
              window.dispatchEvent(
                new CustomEvent("nemefisto:open-add-subscription")
              );
            }}
            aria-label={t("header.addSubscription")}
            title={t("header.addSubscription")}
          >
            <svg
              viewBox="0 0 24 24"
              width="14"
              height="14"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
          </button>
        )}
        {hasDashboardUrl && (
          <button
            type="button"
            className="icon-btn"
            onClick={openDashboard}
            aria-label={t("header.dashboard")}
            title={t("header.dashboard")}
          >
            <UserIcon />
          </button>
        )}
        <button
          type="button"
          className="icon-btn"
          onClick={onOpenSettings}
          aria-label={t("header.settings")}
          title={t("header.settings")}
        >
          <SettingsIcon />
        </button>
      </div>
    </header>
  );
}
