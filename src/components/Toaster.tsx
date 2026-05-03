import { useTranslation } from "react-i18next";
import { useToastStore } from "../stores/toastStore";

/**
 * Контейнер для тостов — монтируется один раз в App.tsx, рендерит
 * стек активных уведомлений в правом нижнем углу. Тосты добавляются
 * через `showToast()` (см. toastStore.ts), уходят сами через
 * `durationMs` или по клику.
 */
export function Toaster() {
  const { t } = useTranslation();
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  return (
    <div className="toaster">
      {toasts.map((toast) => (
        <button
          key={toast.id}
          type="button"
          className={`toast toast-${toast.kind}`}
          onClick={() => dismiss(toast.id)}
          title={t("toaster.dismissTitle")}
        >
          {toast.title && <div className="toast-title">{toast.title}</div>}
          <div className="toast-message">
            {toast.message.split("\n").map((line, i) => (
              <div key={i}>{line}</div>
            ))}
          </div>
        </button>
      ))}
    </div>
  );
}
