import { useTranslation } from "react-i18next";
import { useUtcClock } from "../lib/hooks/useUtcClock";
import { openDashboard, useHasDashboardUrl } from "../lib/openExternal";
import { SettingsIcon, UserIcon } from "./icons";

/**
 * Шапка приложения: лого + UTC-часы + кнопки личного кабинета и настроек.
 */
export function Header({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { t } = useTranslation();
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
