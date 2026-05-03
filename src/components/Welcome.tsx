import { useState, type DragEvent } from "react";
import { useTranslation } from "react-i18next";
import { useSubscriptionStore } from "../stores/subscriptionStore";

/**
 * Карточка первого запуска: ввод URL подписки.
 * Показывается когда `servers.length === 0`.
 *
 * Drag-and-drop: можно бросить ссылку из браузера прямо в карточку —
 * URL парсится из `text/uri-list` (приоритет) или `text/plain`,
 * валидируется как http(s):// и сразу триггерит `fetchSubscription`.
 */
export function Welcome() {
  const { t } = useTranslation();
  const subUrl = useSubscriptionStore((s) => s.url);
  const subLoading = useSubscriptionStore((s) => s.loading);
  const subError = useSubscriptionStore((s) => s.error);
  const setSubUrl = useSubscriptionStore((s) => s.setUrl);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);

  const [dragActive, setDragActive] = useState(false);

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setDragActive(true);
  };
  const onDragLeave = () => setDragActive(false);

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragActive(false);
    const dt = e.dataTransfer;
    if (!dt) return;
    // text/uri-list — формат drag из адресной строки браузера
    // (одна строка = один URL, могут быть multiline). Берём первую.
    let raw = dt.getData("text/uri-list");
    if (raw) {
      raw = raw.split(/\r?\n/).find((l) => l && !l.startsWith("#")) ?? "";
    }
    if (!raw) raw = dt.getData("text/plain");
    raw = raw.trim();
    if (!raw) return;

    // Простая валидация — http(s)://. Иначе оставляем юзеру набрать.
    if (!/^https?:\/\//i.test(raw)) return;

    setSubUrl(raw);
    void fetchSubscription();
  };

  return (
    <div
      className={`welcome${dragActive ? " is-drag-over" : ""}`}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <div className="welcome-tag">— {t("welcome.tag")}</div>
      <h2 className="welcome-title">{t("welcome.title")}</h2>
      <p className="welcome-desc">
        {t("welcome.desc.before")}&nbsp;
        <span className="bracket">https://sub.example.com/...</span>
        {t("welcome.desc.after")}
      </p>
      <p className="welcome-desc welcome-desc-hint">{t("welcome.dropHint")}</p>
      <div className="row-input" style={{ marginTop: 8 }}>
        <input
          type="url"
          autoFocus
          value={subUrl}
          onChange={(e) => setSubUrl(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && fetchSubscription()}
          placeholder="https://sub.example.com/..."
          className="input"
        />
        <button
          type="button"
          disabled={subLoading || !subUrl.trim()}
          onClick={() => fetchSubscription()}
          className="btn-ghost"
        >
          {subLoading ? "…" : t("welcome.load")}
        </button>
      </div>
      {subError && <pre className="hero-error">{subError}</pre>}
    </div>
  );
}
