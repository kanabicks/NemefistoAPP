import { useState } from "react";
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
  const [busy, setBusy] = useState(false);
  const diff = diffBackup(backup);

  const onApply = () => {
    setBusy(true);
    try {
      applyBackup(backup);
      showToast({
        kind: "success",
        title: "настройки применены",
        message: `${diff.length} изменений${
          backup.subscription_url
            ? "\nURL подписки обновлён — нажмите ↻ чтобы перезагрузить серверы"
            : ""
        }`,
      });
      onClose();
    } catch (e) {
      showToast({
        kind: "error",
        title: "не удалось применить",
        message: String(e),
      });
      setBusy(false);
    }
  };

  const exportedAt = backup.exported_at
    ? new Date(backup.exported_at).toLocaleString("ru-RU")
    : "(время не указано)";

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog" style={{ maxWidth: 460 }}>
        <div className="recovery-title">импорт настроек</div>
        <div className="recovery-text">
          backup создан{" "}
          <span style={{ color: "var(--fg)" }}>{exportedAt}</span>, версия
          приложения {backup.app_version}.
        </div>
        {diff.length === 0 ? (
          <div className="recovery-text" style={{ marginTop: 8 }}>
            ничего не изменится — все импортируемые значения уже совпадают
            с текущими.
          </div>
        ) : (
          <>
            <div
              className="recovery-text"
              style={{ marginTop: 8, marginBottom: 6 }}
            >
              изменится <b>{diff.length}</b>{" "}
              {diff.length === 1 ? "настройка" : diff.length < 5 ? "настройки" : "настроек"}:
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
            отмена
          </button>
          <button
            type="button"
            className="btn-primary"
            onClick={onApply}
            disabled={busy || diff.length === 0}
          >
            {busy ? "…" : "применить"}
          </button>
        </div>
      </div>
    </div>
  );
}
