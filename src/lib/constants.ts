import type { VpnStatus } from "../stores/vpnStore";

export const DASHBOARD_URL = "https://web.nemefisto.online";
export const SUPPORT_URL = "https://t.me/nemefistovpn_bot";

// 14.J: версия приложения автоматически прокидывается из package.json
// через vite define (см. vite.config.ts). Bump только в одном месте.
declare const __APP_VERSION__: string;
export const APP_VERSION = __APP_VERSION__;

export const GITHUB_URL = "https://github.com/kanabicks/NemefistoAPP";
export const PRIVACY_URL = `${GITHUB_URL}/blob/main/PRIVACY.md`;
export const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;

/**
 * CSS-классы для status-pill / power-label.
 * Сами тексты — в i18n (`status.pill.*`, `status.label.*`, `mode.*`).
 */
export const STATUS_PILL_CLS: Record<VpnStatus, string> = {
  stopped: "",
  starting: "is-busy",
  running: "is-running",
  stopping: "is-busy",
  error: "is-error",
};

export const POWER_LABEL_CLS: Record<VpnStatus, string> = {
  stopped: "dim",
  starting: "",
  running: "",
  stopping: "dim",
  error: "warn",
};
