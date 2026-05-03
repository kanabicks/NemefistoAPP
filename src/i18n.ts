/**
 * 14.J: i18n config через `react-i18next`.
 *
 * Стратегия:
 * - Два языка: `ru` (default) и `en`.
 * - Источник перевода — статические JSON файлы в `src/locales/{lang}/translation.json`,
 *   импортятся напрямую (без HTTP backend — приложение offline-first, тащить
 *   `i18next-fetch-backend` нет смысла).
 * - Detection: настройка `language` в settingsStore (`auto` | `ru` | `en`).
 *   `auto` → читаем `navigator.language`, начинается на `ru` → `ru`, иначе `en`.
 * - Fallback `en` — если ключ не найден в `ru`, показываем английский.
 *   Для обратного направления — наоборот fallback не делаем (пустая строка
 *   на UI лучше чем «сваленный» английский там где должен быть русский).
 *
 * Главные принципы для разработчика:
 * 1. `useTranslation()` в любом компоненте → `const { t } = useTranslation();
 *    t("path.to.key")`.
 * 2. Ключи nested by feature: `header.title`, `power.connect`, `settings.engine.label`.
 * 3. `Trans`-компонент для строк с inline-разметкой (ссылки, span'ы).
 * 4. Plurals через `count` параметр + `_one/_other` суффиксы в JSON.
 * 5. Hard-coded RU-строки в коде НЕ оставляем — даже однократные. UI-only.
 *    Backend error messages (toast'ы из Rust) пока остаются на русском —
 *    их перевод требует error-code'ов, отложили до следующих релизов.
 */

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import ru from "./locales/ru/translation.json";
import en from "./locales/en/translation.json";

/** Определить начальный язык из настроек или из navigator.language. */
function detectLanguage(stored: string | null): "ru" | "en" {
  if (stored === "ru" || stored === "en") return stored;
  const nav = navigator.language?.toLowerCase() ?? "";
  return nav.startsWith("ru") ? "ru" : "en";
}

/**
 * Прочитать сохранённый выбор языка из localStorage **до** инициализации
 * settingsStore — i18n должен быть готов до первого render'а компонентов.
 */
function readStoredLanguage(): string | null {
  try {
    const raw = localStorage.getItem("nemefisto.settings.v1");
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { language?: string };
    return parsed.language ?? null;
  } catch {
    return null;
  }
}

const initialLng = detectLanguage(readStoredLanguage());

void i18n.use(initReactI18next).init({
  resources: {
    ru: { translation: ru },
    en: { translation: en },
  },
  lng: initialLng,
  fallbackLng: "en",
  interpolation: {
    escapeValue: false, // React сам экранирует
  },
  returnNull: false,
});

export default i18n;
