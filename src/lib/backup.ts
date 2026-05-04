import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import i18n from "../i18n";
import { useSettingsStore, type Settings, type AppRule } from "../stores/settingsStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { APP_VERSION } from "./constants";

/**
 * 12.D — backup/restore настроек.
 *
 * Сериализуем подмножество Settings + URL подписки + appRules в JSON.
 * **Whitelist**: HWID-override, dismissed-set объявлений и localStorage
 * tutorial-флаги наружу не идут (machine-specific / UX-state, не должны
 * переноситься между устройствами).
 *
 * Импорт защищён через `validate()` — отсев невалидных enum-значений
 * и неожиданных типов.
 */

export type BackupSchema = {
  schema_version: 1;
  app_version: string;
  exported_at: number; // unix-ms
  settings: Partial<Settings>;
  subscription_url: string;
  app_rules: AppRule[];
};

/** Поля Settings которые мы сохраняем при экспорте/импорте.
 *
 *  Не вошли:
 *   - все touched-флаги (восстанавливаем из значений сами);
 *   - nothing else — основные поля все попадают сюда. */
const SETTINGS_WHITELIST: Array<keyof Settings> = [
  "autoRefresh",
  "autoRefreshHours",
  "refreshOnOpen",
  "pingOnOpen",
  "connectOnOpen",
  "sendHwid",
  "userAgent",
  "sort",
  "allowLan",
  "theme",
  "background",
  "buttonStyle",
  "preset",
  "antiDpiFragmentation",
  "antiDpiFragmentationPackets",
  "antiDpiFragmentationLength",
  "antiDpiFragmentationInterval",
  "antiDpiNoises",
  "antiDpiNoisesType",
  "antiDpiNoisesPacket",
  "antiDpiNoisesDelay",
  "antiDpiServerResolve",
  "antiDpiResolveDoH",
  "antiDpiResolveBootstrap",
  "tunMasking",
  "tunOnlyStrict",
  "killSwitch",
  "killSwitchStrict",
  "autoApplyMinimalRuRules",
  "dnsLeakProtection",
  "forceDisableIpv6",
  "pingMethod",
  "pingUrl",
  "pingTimeoutSec",
  "showMemoryMonitor",
  "mux",
  "muxProtocol",
  "muxMaxStreams",
  "engine",
  "shortcutToggleVpn",
  "shortcutShowHide",
  "shortcutSwitchMode",
  "floatingWindow",
  "autoLeakTest",
  "trustedSsids",
  "trustedSsidAction",
  "autoConnectOnLeave",
];

/** Собрать backup-объект из текущих store'ов. */
export function collectBackup(): BackupSchema {
  const s = useSettingsStore.getState();
  const sub = useSubscriptionStore.getState();
  const settings: Partial<Settings> = {};
  for (const key of SETTINGS_WHITELIST) {
    // Type-assertion необходима — TS не выводит общий тип Settings[K]
    // через цикл по разнотипным ключам.
    (settings as Record<string, unknown>)[key as string] = s[key];
  }
  return {
    schema_version: 1,
    app_version: APP_VERSION,
    exported_at: Date.now(),
    settings,
    subscription_url: sub.url,
    app_rules: s.appRules,
  };
}

/** Сохранить backup-файл в `~/Documents/`. Возвращает путь. */
export async function exportBackupToDocuments(): Promise<string> {
  const backup = collectBackup();
  const json = JSON.stringify(backup, null, 2);
  return await invoke<string>("export_settings_to_documents", { json });
}

/** Скачать backup по URL (для deep-link import-from-url). */
export async function fetchBackupFromUrl(url: string): Promise<string> {
  return await invoke<string>("fetch_settings_backup", { url });
}

/** Прочитать локальный File через FileReader (для file-input). */
export function readBackupFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("FileReader error"));
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.readAsText(file);
  });
}

/** Безопасный парсер JSON-payload в `BackupSchema` или ошибка. */
export function parseBackup(raw: string): BackupSchema {
  let obj: unknown;
  try {
    obj = JSON.parse(raw);
  } catch (e) {
    throw new Error(i18n.t("backup.parseError", { error: String(e) }));
  }
  if (!obj || typeof obj !== "object") {
    throw new Error(i18n.t("backup.expectedJson"));
  }
  const o = obj as Record<string, unknown>;
  if (o.schema_version !== 1) {
    throw new Error(
      i18n.t("backup.unsupportedSchema", { version: String(o.schema_version) })
    );
  }
  const settings = o.settings;
  const sub = typeof o.subscription_url === "string" ? o.subscription_url : "";
  const rulesRaw = Array.isArray(o.app_rules) ? o.app_rules : [];
  const rules: AppRule[] = rulesRaw
    .filter(
      (r): r is { exe: string; action: AppRule["action"]; comment?: string } =>
        !!r &&
        typeof r === "object" &&
        typeof (r as Record<string, unknown>).exe === "string" &&
        ((r as Record<string, unknown>).action === "proxy" ||
          (r as Record<string, unknown>).action === "direct" ||
          (r as Record<string, unknown>).action === "block")
    )
    .map((r) => ({
      exe: r.exe.trim(),
      action: r.action,
      comment: typeof r.comment === "string" ? r.comment : undefined,
    }));

  return {
    schema_version: 1,
    app_version: typeof o.app_version === "string" ? o.app_version : "?",
    exported_at:
      typeof o.exported_at === "number" && Number.isFinite(o.exported_at)
        ? o.exported_at
        : 0,
    settings: settings && typeof settings === "object" ? (settings as Partial<Settings>) : {},
    subscription_url: sub,
    app_rules: rules,
  };
}

/** Применить backup к store'ам. Touched-флаги выставляем там где должно
 *  «прилипнуть» (engine/theme/etc) — иначе server-driven заголовки тут
 *  же перебьют импортируемое значение, и пользователь не получит то
 *  что ожидал. */
export function applyBackup(backup: BackupSchema): void {
  const s = useSettingsStore.getState();
  for (const key of SETTINGS_WHITELIST) {
    const incoming = (backup.settings as Record<string, unknown>)[key as string];
    if (incoming === undefined) continue;
    // set() сам пробрасывает touched-флаги для themeTouched / engineTouched
    // и т.п. — переиспользуем эту логику.
    s.set(key, incoming as never);
  }
  // appRules — отдельный setter (не один из ключей выше).
  useSettingsStore.getState().set("appRules", backup.app_rules);

  if (backup.subscription_url.trim()) {
    useSubscriptionStore.getState().setUrl(backup.subscription_url.trim());
  }
}

/** Diff между текущим store и импортируемым backup'ом. Возвращает список
 *  «ключ: текущее → импортируемое» — для preview-modal'а. */
export type BackupDiffEntry = {
  key: string;
  current: string;
  incoming: string;
};

export function diffBackup(backup: BackupSchema): BackupDiffEntry[] {
  const s = useSettingsStore.getState();
  const sub = useSubscriptionStore.getState();
  const out: BackupDiffEntry[] = [];

  const fmt = (v: unknown): string => {
    if (v === null || v === undefined) return i18n.t("backup.values.dash");
    if (typeof v === "boolean")
      return v ? i18n.t("backup.values.on") : i18n.t("backup.values.off");
    if (Array.isArray(v))
      return v.length === 0
        ? i18n.t("backup.values.empty")
        : i18n.t("backup.values.itemsCount", { count: v.length });
    return String(v);
  };

  for (const key of SETTINGS_WHITELIST) {
    const incoming = (backup.settings as Record<string, unknown>)[key as string];
    if (incoming === undefined) continue;
    const current = s[key];
    // Простое сравнение строкой через JSON для массивов и enum'ов.
    if (JSON.stringify(current) !== JSON.stringify(incoming)) {
      out.push({ key: String(key), current: fmt(current), incoming: fmt(incoming) });
    }
  }

  if (
    backup.subscription_url.trim() &&
    backup.subscription_url.trim() !== sub.url.trim()
  ) {
    out.push({
      key: i18n.t("backup.fieldLabels.subscriptionUrl"),
      current: sub.url
        ? sub.url.slice(0, 60) + (sub.url.length > 60 ? "…" : "")
        : i18n.t("backup.values.dash"),
      incoming:
        backup.subscription_url.slice(0, 60) +
        (backup.subscription_url.length > 60 ? "…" : ""),
    });
  }

  // appRules сравниваем отдельно — только количество / наличие.
  if (JSON.stringify(s.appRules) !== JSON.stringify(backup.app_rules)) {
    out.push({
      key: i18n.t("backup.fieldLabels.appRules"),
      current: i18n.t("backup.values.itemsCount", { count: s.appRules.length }),
      incoming: i18n.t("backup.values.itemsCount", {
        count: backup.app_rules.length,
      }),
    });
  }

  return out;
}

/** Глобальный store для backup-preview модалки. App.tsx рендерит
 *  `<BackupPreviewModal>` если `pending != null`. */
type BackupModalStore = {
  pending: BackupSchema | null;
  show: (b: BackupSchema) => void;
  close: () => void;
};

export const useBackupModalStore = create<BackupModalStore>((set) => ({
  pending: null,
  show: (b) => set({ pending: b }),
  close: () => set({ pending: null }),
}));
