import { useEffect, useState } from "react";

/**
 * 14.G — first-run onboarding (модалка-туториал из 4 шагов).
 *
 * Показывается ровно один раз — после первого пройденного шага флаг
 * `nemefisto.onboarding.completed.v1` сохраняется в localStorage и при
 * следующих запусках модалка не появляется.
 *
 * Предполагается что условие «первый запуск» проверяет родитель —
 * передаёт `open=true` если флага нет в localStorage. Кнопка
 * «пропустить» / финальный «готово» — единственные пути закрыть
 * модалку, оба ставят флаг.
 *
 * Дизайн: оверлей с blur-фоном, центральная карточка ~360 px, прогресс
 * точками. Использует те же CSS-классы что `recovery-overlay/dialog`
 * для консистентности.
 */

const STORAGE_KEY = "nemefisto.onboarding.completed.v1";

type Step = {
  title: string;
  body: React.ReactNode;
};

const STEPS: Step[] = [
  {
    title: "добро пожаловать в nemefisto",
    body: (
      <>
        <p style={{ marginBottom: 8 }}>
          VPN-клиент на двух ядрах (xray + mihomo) с защитой от DPI,
          утечек и локального детекта. весь код открытый, никакой
          телеметрии.
        </p>
        <p style={{ color: "var(--fg-dim)" }}>
          краткий тур из 3 шагов — займёт минуту.
        </p>
      </>
    ),
  },
  {
    title: "1 · добавь подписку",
    body: (
      <>
        <p style={{ marginBottom: 8 }}>
          получи URL подписки у своего VPN-провайдера и вставь в поле
          на главном экране. формат:{" "}
          <span className="bracket">https://sub.example.com/...</span>
        </p>
        <p style={{ color: "var(--fg-dim)" }}>
          поддерживаются Marzban, 3x-ui, sing-box, base64-списки,
          Mihomo YAML — всё что отдают современные панели.
        </p>
      </>
    ),
  },
  {
    title: "2 · выбери сервер и подключайся",
    body: (
      <>
        <p style={{ marginBottom: 8 }}>
          приложение скачает список серверов и сразу замерит до них
          пинги. тапни выпадающий список под кнопкой питания и выбери
          любой — рядом с именем будет latency.
        </p>
        <p style={{ color: "var(--fg-dim)" }}>
          большая круглая кнопка — connect / disconnect. подключение
          обычно занимает 1-2 секунды.
        </p>
      </>
    ),
  },
  {
    title: "3 · готово",
    body: (
      <>
        <p style={{ marginBottom: 8 }}>
          настройки в правом верхнем углу: kill switch, anti-DPI,
          темы оформления, маршрутизация по странам, доверенные wi-fi
          сети.
        </p>
        <p style={{ color: "var(--fg-dim)" }}>
          горячие клавиши: <span className="bracket">Ctrl+Shift+V</span>{" "}
          — toggle VPN, <span className="bracket">Ctrl+Shift+M</span>{" "}
          — показать/скрыть окно.
        </p>
      </>
    ),
  },
];

/** Прочитать флаг «онбординг уже пройден». Используется родителем
 *  чтобы решать показывать ли модалку. */
export function isOnboardingCompleted(): boolean {
  try {
    return !!localStorage.getItem(STORAGE_KEY);
  } catch {
    // приватный режим / квота — считаем что пройден, чтобы не доставать.
    return true;
  }
}

function markCompleted() {
  try {
    localStorage.setItem(STORAGE_KEY, "1");
  } catch {
    // ignore
  }
}

export function OnboardingTour({ onClose }: { onClose: () => void }) {
  const [step, setStep] = useState(0);
  const total = STEPS.length;

  // Esc — пропустить тур.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        markCompleted();
        onClose();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const isLast = step === total - 1;
  const onNext = () => {
    if (isLast) {
      markCompleted();
      onClose();
    } else {
      setStep((s) => s + 1);
    }
  };
  const onSkip = () => {
    markCompleted();
    onClose();
  };

  const cur = STEPS[step];

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog" style={{ maxWidth: 380 }}>
        <div className="recovery-title">{cur.title}</div>
        <div className="recovery-text" style={{ minHeight: 110 }}>
          {cur.body}
        </div>

        {/* Прогресс точками */}
        <div
          style={{
            display: "flex",
            justifyContent: "center",
            gap: 6,
            margin: "12px 0",
          }}
        >
          {STEPS.map((_, i) => (
            <span
              key={i}
              aria-hidden
              style={{
                width: 6,
                height: 6,
                borderRadius: "50%",
                background:
                  i === step
                    ? "var(--fg)"
                    : i < step
                    ? "var(--fg-dim)"
                    : "var(--border)",
                transition: "background 0.2s",
              }}
            />
          ))}
        </div>

        <div className="recovery-actions">
          <button
            type="button"
            className="btn-ghost"
            onClick={onSkip}
            disabled={isLast}
            style={{ visibility: isLast ? "hidden" : "visible" }}
          >
            пропустить
          </button>
          {step > 0 && (
            <button
              type="button"
              className="btn-ghost"
              onClick={() => setStep((s) => s - 1)}
            >
              назад
            </button>
          )}
          <button type="button" className="btn-primary" onClick={onNext}>
            {isLast ? "готово" : "далее"}
          </button>
        </div>
      </div>
    </div>
  );
}
