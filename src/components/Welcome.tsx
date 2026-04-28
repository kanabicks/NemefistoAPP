import { useSubscriptionStore } from "../stores/subscriptionStore";
import { openDashboard } from "../lib/openExternal";

/**
 * Карточка первого запуска: ввод URL подписки или вход в личный кабинет.
 * Показывается когда `servers.length === 0`.
 */
export function Welcome() {
  const subUrl = useSubscriptionStore((s) => s.url);
  const subLoading = useSubscriptionStore((s) => s.loading);
  const subError = useSubscriptionStore((s) => s.error);
  const setSubUrl = useSubscriptionStore((s) => s.setUrl);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);

  return (
    <div className="welcome">
      <div className="welcome-tag">— подключение за минуту</div>
      <h2 className="welcome-title">добавь подписку</h2>
      <p className="welcome-desc">
        вставь ссылку на свою подписку (URL вида&nbsp;
        <span className="bracket">https://sub.example.com/...</span>),
        приложение скачает список серверов и сразу замерит до них пинги.
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
          {subLoading ? "…" : "загрузить"}
        </button>
      </div>
      {subError && <pre className="hero-error">{subError}</pre>}
      <div className="welcome-divider">
        <span>или</span>
      </div>
      <button
        type="button"
        onClick={openDashboard}
        className="btn-ghost"
        style={{ alignSelf: "stretch", padding: "12px" }}
      >
        войти в личный кабинет →
      </button>
      <p className="hint" style={{ marginTop: 4 }}>
        web.nemefisto.online · откроется в браузере
      </p>
    </div>
  );
}
