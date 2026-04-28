import { useUtcClock } from "../lib/hooks/useUtcClock";
import { openDashboard } from "../lib/openExternal";
import { SettingsIcon, UserIcon } from "./icons";

/**
 * Шапка приложения: лого + UTC-часы + кнопки личного кабинета и настроек.
 */
export function Header({ onOpenSettings }: { onOpenSettings: () => void }) {
  const utcTime = useUtcClock();

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
        <button
          type="button"
          className="icon-btn"
          onClick={openDashboard}
          aria-label="личный кабинет"
          title="личный кабинет (web.nemefisto.online)"
        >
          <UserIcon />
        </button>
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
