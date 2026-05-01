import { useUtcClock } from "../lib/hooks/useUtcClock";
import { openDashboard, useHasDashboardUrl } from "../lib/openExternal";
import { SettingsIcon, UserIcon } from "./icons";

/**
 * Шапка приложения: лого + UTC-часы + кнопки личного кабинета и настроек.
 *
 * Кнопка личного кабинета показывается ТОЛЬКО если подписка прислала
 * `profile-web-page-url` — без хедера кнопки нет (захардкоженный
 * fallback на нашу страницу убран).
 */
export function Header({ onOpenSettings }: { onOpenSettings: () => void }) {
  const utcTime = useUtcClock();
  const hasDashboardUrl = useHasDashboardUrl();

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
        {hasDashboardUrl && (
          <button
            type="button"
            className="icon-btn"
            onClick={openDashboard}
            aria-label="личный кабинет"
            title="личный кабинет"
          >
            <UserIcon />
          </button>
        )}
        <button
          type="button"
          className="icon-btn"
          onClick={onOpenSettings}
          aria-label="настройки"
          title="настройки"
        >
          <SettingsIcon />
        </button>
      </div>
    </header>
  );
}
