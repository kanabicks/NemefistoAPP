import type { VpnMode, VpnStatus } from "../stores/vpnStore";

export const DASHBOARD_URL = "https://web.nemefisto.online";
export const SUPPORT_URL = "https://t.me/nemefistovpn_bot";
export const APP_VERSION = "0.1.0";

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
