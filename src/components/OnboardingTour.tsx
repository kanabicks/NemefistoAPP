import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

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

const STEP_KEYS = ["step1", "step2", "step3", "step4"] as const;

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

function StepBody({ step }: { step: (typeof STEP_KEYS)[number] }) {
  const { t } = useTranslation();
  switch (step) {
    case "step1":
      return (
        <>
          <p style={{ marginBottom: 8 }}>{t("onboarding.step1.body")}</p>
          <p style={{ color: "var(--fg-dim)" }}>
            {t("onboarding.step1.footnote")}
          </p>
        </>
      );
    case "step2":
      return (
        <>
          <p style={{ marginBottom: 8 }}>
            {t("onboarding.step2.bodyBefore")}
            <span className="bracket">https://sub.example.com/...</span>
          </p>
          <p style={{ color: "var(--fg-dim)" }}>
            {t("onboarding.step2.footnote")}
          </p>
        </>
      );
    case "step3":
      return (
        <>
          <p style={{ marginBottom: 8 }}>{t("onboarding.step3.body")}</p>
          <p style={{ color: "var(--fg-dim)" }}>
            {t("onboarding.step3.footnote")}
          </p>
        </>
      );
    case "step4":
      return (
        <>
          <p style={{ marginBottom: 8 }}>{t("onboarding.step4.body")}</p>
          <p style={{ color: "var(--fg-dim)" }}>
            {t("onboarding.step4.footnoteBefore")}
            <span className="bracket">Ctrl+Shift+V</span>
            {t("onboarding.step4.footnoteToggleVpn")}
            <span className="bracket">Ctrl+Shift+M</span>
            {t("onboarding.step4.footnoteShowHide")}
          </p>
        </>
      );
  }
}

export function OnboardingTour({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [step, setStep] = useState(0);
  const total = STEP_KEYS.length;

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

  const stepKey = STEP_KEYS[step];
  const title = t(`onboarding.${stepKey}.title`);

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog" style={{ maxWidth: 380 }}>
        <div className="recovery-title">{title}</div>
        <div className="recovery-text" style={{ minHeight: 110 }}>
          <StepBody step={stepKey} />
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
          {STEP_KEYS.map((_, i) => (
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
            {t("onboarding.skip")}
          </button>
          {step > 0 && (
            <button
              type="button"
              className="btn-ghost"
              onClick={() => setStep((s) => s - 1)}
            >
              {t("onboarding.back")}
            </button>
          )}
          <button type="button" className="btn-primary" onClick={onNext}>
            {isLast ? t("onboarding.done") : t("onboarding.next")}
          </button>
        </div>
      </div>
    </div>
  );
}
