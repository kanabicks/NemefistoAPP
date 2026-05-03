import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

/**
 * Превью спарсенного и сгенерированного конфига сервера.
 *
 * Открывается клавишей-стрелкой `>` на server-row. Полезно для отладки:
 * пользователь видит что именно подписка прислала и что мы подсунем
 * движку при connect. Mirrors поведение Happ-клиента.
 */
type ServerPreview = {
  name: string;
  protocol: string;
  server: string;
  port: number;
  engine_compat: string[];
  raw: string;
  /** sing-box JSON если URI/xray-json/singbox-json; `null` для mihomo-profile (там raw — это YAML). */
  generated_singbox: string | null;
};

type Tab = "raw" | "generated";

export function ServerPreviewModal({
  serverIndex,
  onClose,
}: {
  serverIndex: number;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<ServerPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("generated");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setError(null);
    invoke<ServerPreview>("preview_server_config", { serverIndex })
      .then((data) => {
        if (cancelled) return;
        setPreview(data);
        // Если для этого entry sing-box-конфиг не генерируется
        // (mihomo-profile) — сразу переключаемся на raw-таб.
        if (!data.generated_singbox) setTab("raw");
      })
      .catch((e) => {
        if (cancelled) return;
        setError(typeof e === "string" ? e : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [serverIndex]);

  // Esc для закрытия
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const content =
    tab === "generated" ? preview?.generated_singbox ?? "" : preview?.raw ?? "";

  const copy = async () => {
    if (!content) return;
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // приватный режим / старый webview — игнорируем
    }
  };

  return (
    <div className="preview-modal-backdrop" onClick={onClose}>
      <div
        className="preview-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="preview-modal-header">
          <div className="preview-modal-title">
            {preview ? (
              <>
                <div className="preview-modal-name">{preview.name}</div>
                <div className="preview-modal-meta">
                  {preview.protocol.toUpperCase()} · {preview.server}
                  {preview.port ? `:${preview.port}` : ""}
                </div>
              </>
            ) : (
              <div className="preview-modal-name">{t("modal.serverPreview.title")}</div>
            )}
          </div>
          <button
            className="preview-modal-close"
            onClick={onClose}
            aria-label={t("modal.serverPreview.closeAria")}
          >
            ×
          </button>
        </div>

        {preview && preview.generated_singbox && (
          <div className="preview-modal-tabs">
            <button
              className={`preview-modal-tab${tab === "generated" ? " is-active" : ""}`}
              onClick={() => setTab("generated")}
            >
              {t("modal.serverPreview.tabGenerated")}
            </button>
            <button
              className={`preview-modal-tab${tab === "raw" ? " is-active" : ""}`}
              onClick={() => setTab("raw")}
            >
              {t("modal.serverPreview.tabRaw")}
            </button>
          </div>
        )}

        <div className="preview-modal-body">
          {error ? (
            <div className="preview-modal-error">
              {t("modal.serverPreview.errorPrefix", { message: error })}
            </div>
          ) : !preview ? (
            <div className="preview-modal-loading">
              {t("modal.serverPreview.loading")}
            </div>
          ) : (
            <pre className="preview-modal-pre">
              <code>{content}</code>
            </pre>
          )}
        </div>

        {preview && content && (
          <div className="preview-modal-footer">
            <button className="btn-secondary" onClick={copy}>
              {copied
                ? t("modal.serverPreview.copied")
                : t("modal.serverPreview.copy")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
