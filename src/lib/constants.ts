import type { VpnMode, VpnStatus } from "../stores/vpnStore";

export const DASHBOARD_URL = "https://web.nemefisto.online";
export const SUPPORT_URL = "https://t.me/nemefistovpn_bot";
export const APP_VERSION = "0.1.0";

export const GITHUB_URL = "https://github.com/kanabicks/NemefistoAPP";
export const PRIVACY_URL = `${GITHUB_URL}/blob/main/PRIVACY.md`;
export const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;

export const STATUS_PILL: Record<VpnStatus, { label: string; cls: string }> = {
  stopped: { label: "STANDBY", cls: "" },
  starting: { label: "ПОДКЛЮЧЕНИЕ", cls: "is-busy" },
  running: { label: "TUNNEL UP", cls: "is-running" },
  stopping: { label: "ОТКЛЮЧЕНИЕ", cls: "is-busy" },
  error: { label: "ERROR", cls: "is-error" },
};

export const POWER_LABEL: Record<VpnStatus, { text: string; cls: string }> = {
  stopped: { text: "не подключён", cls: "dim" },
  starting: { text: "подключаемся…", cls: "" },
  running: { text: "защищён", cls: "" },
  stopping: { text: "отключаемся…", cls: "dim" },
  error: { text: "ошибка", cls: "warn" },
};

export const MODE_LABEL: Record<VpnMode, string> = {
  proxy: "системный прокси",
  tun: "tun (весь трафик)",
};
