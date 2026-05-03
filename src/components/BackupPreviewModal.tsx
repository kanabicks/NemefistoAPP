import { useState } from "react";
import { useTranslation } from "react-i18next";
import { applyBackup, diffBackup, type BackupSchema } from "../lib/backup";
import { showToast } from "../stores/toastStore";

/**
 * 12.D — preview-модалка для импорта backup'а.
 *
 * Показывает diff (что изменится) и две кнопки. До нажатия «применить»
 * никакие настройки не меняются.
 */
export function BackupPreviewModal({
  backup,
  onClose,
}: {
  backup: BackupSchema;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const diff = diffBackup(backup);

  const onApply = () => {
    setBusy(true);
    try {
      applyBackup(backup);
      const suffix = backup.subscription_url
        ? t("modal.backup.subscriptionUpdatedSuffix")
        : "";
      showToast({
        kind: "success",
        title: t("modal.backup.appliedTitle"),
        message: `${t("modal.backup.appliedMessage", { count: diff.length })}${suffix}`,
      });
      onClose();
    } catch (e) {
      showToast({
        kind: "error",
        title: t("modal.backup.applyFailedTitle"),
        message: String(e),
      });
      setBusy(false);
    }
  };

  const localeTag = i18n.language === "ru" ? "ru-RU" : "en-US";
  const exportedAt = backup.exported_at
    ? new Date(backup.exported_at).toLocaleString(localeTag)
    : t("modal.backup.timeNotSet");

  // Подобрать форму "изменится N настроек" в зависимости от языка/числа.
  const lang = i18n.language;
  let changesPrefixKey: string;
  if (lang === "ru") {
    if (diff.length === 1) changesPrefixKey = "modal.backup.changesPrefixOne";
    else if (diff.length > 1 && diff.length < 5)
      changesPrefixKey = "modal.backup.changesPrefixFew";
    else changesPrefixKey = "modal.backup.changesPrefixMany";
  } else {
    changesPrefixKey =
      diff.length === 1
        ? "modal.backup.changesPrefixOne"
        : "modal.backup.changesPrefixOther";
  }
  const changesPrefix = t(changesPrefixKey, { count: diff.length });
  // Простой парсер плейсхолдера <1>...</1> чтобы оборачивать число в <b>.
  const renderChangesPrefix = () => {
    const m = changesPrefix.match(/^(.*?)<1>([^<]*)<\/1>(.*)$/);
    if (!m) return changesPrefix;
    const [, before, inner, after] = m;
    return (
      <>
        {before}
        <b>{inner}</b>
        {after}
      </>
    );
  };

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog" style={{ maxWidth: 460 }}>
        <div className="recovery-title">{t("modal.backup.title")}</div>
        <div className="recovery-text">
          {t("modal.backup.createdAtPrefix")}
          <span style={{ color: "var(--fg)" }}>{exportedAt}</span>
          {t("modal.backup.createdAtMiddle", { version: backup.app_version })}
        </div>
        {diff.length === 0 ? (
          <div className="recovery-text" style={{ marginTop: 8 }}>
            {t("modal.backup.noChanges")}
          </div>
        ) : (
          <>
            <div
              className="recovery-text"
              style={{ marginTop: 8, marginBottom: 6 }}
            >
              {renderChangesPrefix()}
            </div>
            <ul
              className="recovery-list"
              style={{
                maxHeight: 240,
                overflowY: "auto",
                fontSize: 12,
                fontFamily: "var(--font-mono, monospace)",
              }}
            >
              {diff.map((d) => (
                <li key={d.key}>
                  <span style={{ color: "var(--fg-dim)" }}>{d.key}:</span>{" "}
                  <span style={{ color: "var(--fg-dim)" }}>{d.current}</span>{" "}
                  <span style={{ color: "var(--fg-dim)" }}>→</span>{" "}
                  <span style={{ color: "var(--fg)" }}>{d.incoming}</span>
                </li>
              ))}
            </ul>
          </>
        )}
        <div className="recovery-actions">
          <button
            type="button"
            className="btn-ghost"
            onClick={onClose}
            disabled={busy}
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="btn-primary"
            onClick={onApply}
            disabled={busy || diff.length === 0}
          >
            {busy ? "…" : t("common.apply")}
          </button>
        </div>
      </div>
    </div>
  );
}
