import { useTranslation } from "react-i18next";
import { useSubscriptionStore } from "../stores/subscriptionStore";

/**
 * Карточка первого запуска: ввод URL подписки.
 * Показывается когда `servers.length === 0`.
 */
export function Welcome() {
  const { t } = useTranslation();
  const subUrl = useSubscriptionStore((s) => s.url);
  const subLoading = useSubscriptionStore((s) => s.loading);
  const subError = useSubscriptionStore((s) => s.error);
  const setSubUrl = useSubscriptionStore((s) => s.setUrl);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);

  return (
    <div className="welcome">
      <div className="welcome-tag">— {t("welcome.tag")}</div>
      <h2 className="welcome-title">{t("welcome.title")}</h2>
      <p className="welcome-desc">
        {t("welcome.desc.before")}&nbsp;
        <span className="bracket">https://sub.example.com/...</span>
        {t("welcome.desc.after")}
      </p>
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
