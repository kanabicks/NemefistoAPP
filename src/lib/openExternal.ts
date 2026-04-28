import { openUrl } from "@tauri-apps/plugin-opener";
import { DASHBOARD_URL, SUPPORT_URL } from "./constants";

export function openDashboard() {
  void openUrl(DASHBOARD_URL);
}

export function openSupport() {
  void openUrl(SUPPORT_URL);
}
