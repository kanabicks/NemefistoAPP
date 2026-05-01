import { useToastStore } from "../stores/toastStore";

/**
 * Контейнер для тостов — монтируется один раз в App.tsx, рендерит
 * стек активных уведомлений в правом нижнем углу. Тосты добавляются
 * через `showToast()` (см. toastStore.ts), уходят сами через
 * `durationMs` или по клику.
 */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  return (
    <div className="toaster">
      {toasts.map((t) => (
        <button
          key={t.id}
          type="button"
          className={`toast toast-${t.kind}`}
          onClick={() => dismiss(t.id)}
          title="нажми чтобы скрыть"
        >
          {t.title && <div className="toast-title">{t.title}</div>}
          <div className="toast-message">
            {t.message.split("\n").map((line, i) => (
              <div key={i}>{line}</div>
            ))}
          </div>
        </button>
      ))}
    </div>
  );
}
