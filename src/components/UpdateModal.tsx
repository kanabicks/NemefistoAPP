import { useState } from "react";
import { useUpdateStore } from "../stores/updateStore";
import { useSettingsStore } from "../stores/settingsStore";
import { downloadAndInstall } from "../lib/updater";
import { showToast } from "../stores/toastStore";

/**
 * 14.A: модалка предложения обновления.
 *
 * Открывается когда `useAutoUpdateCheck` нашёл новую версию. Юзер
 * выбирает между «обновить сейчас», «позже» (dismiss этой версии до
 * следующей) и закрытием. При «обновить» — переключаемся в
 * `downloading` state, показываем прогресс-бар, после успешной
 * установки приложение перезапускается автоматически.
 */
export function UpdateModal() {
  const state = useUpdateStore((s) => s.state);
  const setState = useUpdateStore((s) => s.setState);
  const dismissedSet = useSettingsStore((s) => s.set);
  const dismissedList = useSettingsStore((s) => s.dismissedUpdateVersions);
  const [progress, setProgress] = useState(0);

  if (state.kind !== "available" && state.kind !== "downloading") {
    return null;
  }

  const update = state.update;
  const isDownloading = state.kind === "downloading";

  const onDismiss = () => {
    if (isDownloading) return;
    if (!dismissedList.includes(update.version)) {
      dismissedSet("dismissedUpdateVersions", [
        ...dismissedList,
        update.version,
      ]);
    }
    setState({ kind: "idle" });
  };

  const onInstall = async () => {
    setState({ kind: "downloading", update, progress: 0 });
    try {
      await downloadAndInstall(update, (fraction) => {
        setProgress(fraction);
        setState({ kind: "downloading", update, progress: fraction });
      });
      // relaunch() в downloadAndInstall — сюда обычно не доходим,
      // app уже перезапустился. На случай fallback'а:
      setState({ kind: "installed" });
    } catch (e) {
      showToast({
        kind: "error",
        title: "обновление не удалось",
        message: String(e),
      });
      setState({ kind: "idle" });
    }
  };

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog" style={{ maxWidth: 460 }}>
        <div className="recovery-title">
          доступна версия {update.version}
        </div>
        <div className="recovery-text">
          текущая версия:{" "}
          <span style={{ color: "var(--fg)" }}>{update.currentVersion}</span>
        </div>
        {update.notes ? (
          <pre
            className="recovery-text"
            style={{
              marginTop: 12,
              maxHeight: 200,
              overflowY: "auto",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              fontFamily: "var(--font-mono, monospace)",
              fontSize: 12,
              padding: 8,
              background: "var(--bg-soft, rgba(255,255,255,0.04))",
              borderRadius: 6,
            }}
          >
            {update.notes.trim()}
          </pre>
        ) : null}
        {isDownloading ? (
          <div style={{ marginTop: 16 }}>
            <div
              className="recovery-text"
              style={{ marginBottom: 6, fontSize: 12 }}
            >
              скачиваю обновление… {Math.round(progress * 100)}%
            </div>
            <div
              style={{
                height: 6,
                background: "var(--bg-soft, rgba(255,255,255,0.06))",
                borderRadius: 3,
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  width: `${progress * 100}%`,
                  height: "100%",
                  background: "var(--accent, #5cc6c6)",
                  transition: "width 120ms linear",
                }}
              />
            </div>
          </div>
        ) : null}
        <div className="recovery-actions" style={{ marginTop: 16 }}>
          <button
            type="button"
            className="btn-ghost"
            onClick={onDismiss}
            disabled={isDownloading}
          >
            позже
          </button>
          <button
            type="button"
            className="btn-primary"
            onClick={onInstall}
            disabled={isDownloading}
          >
            {isDownloading ? "…" : "обновить и перезапустить"}
          </button>
        </div>
      </div>
    </div>
  );
}
