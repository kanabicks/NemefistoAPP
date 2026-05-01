import { useSubscriptionStore } from "../stores/subscriptionStore";

/**
 * Карточка первого запуска: ввод URL подписки.
 * Показывается когда `servers.length === 0`.
 *
 * Кнопка «войти в личный кабинет» убрана: до загрузки подписки мы не
 * знаем `profile-web-page-url`, а захардкоженный fallback на нашу
 * страницу для универсального клиента некорректен. После загрузки
 * подписки с webPageUrl кнопка появится в Header / основном UI.
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
    </div>
  );
}
