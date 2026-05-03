import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useSubscriptionStore } from "../stores/subscriptionStore";

/**
 * Баннер с объявлением от провайдера подписки. Источник — заголовки
 * `announce` (текст) и `announce-url` (опциональная ссылка).
 *
 * Dismissed-set хранится в localStorage по хешу текста: каждое уникальное
 * объявление пользователь закрывает один раз, а новое (с другим текстом)
 * снова появится. Лимит — 32 хеша, чтобы set не разрастался бесконечно.
 */

const STORAGE_KEY = "nemefisto.dismissed-announces";
const DISMISSED_MAX = 32;

/** Дешёвый стабильный хеш строки (FNV-1a). 8 hex-символов достаточно. */
function hashString(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, "0");
}

function loadDismissed(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((x): x is string => typeof x === "string");
  } catch {
    return [];
  }
}

function saveDismissed(set: string[]) {
  try {
    // Оставляем последние DISMISSED_MAX
    const trimmed = set.slice(-DISMISSED_MAX);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(trimmed));
  } catch {
    // приватный режим — игнорируем
  }
}

export function AnnounceBanner() {
  const { t } = useTranslation();
  const meta = useSubscriptionStore((s) => s.meta);
  const [dismissed, setDismissed] = useState<string[]>(() => loadDismissed());

  const text = meta?.announce ?? null;
  const url = meta?.announceUrl ?? null;
  const hash = text ? hashString(text) : null;
  const isDismissed = hash ? dismissed.includes(hash) : false;

  // Если объявление новое (после переподписки) — оно автоматически появится,
  // т.к. hash будет отсутствовать в dismissed-set. Никаких side-effect'ов
  // на mount: state читается синхронно из localStorage.

  useEffect(() => {
    // Пересинхронизация на случай если другая вкладка изменила storage
    // (для desktop неактуально, но бесплатно).
    const onStorage = (e: StorageEvent) => {
      if (e.key === STORAGE_KEY) setDismissed(loadDismissed());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  if (!text || !hash || isDismissed) return null;

  const onClose = () => {
    const next = [...dismissed.filter((h) => h !== hash), hash];
    setDismissed(next);
    saveDismissed(next);
  };

  const onClick = () => {
    if (url) void openUrl(url);
  };

  return (
    <div className={`announce-banner${url ? " is-clickable" : ""}`}>
      <button
        type="button"
        className="announce-banner-body"
        onClick={onClick}
        disabled={!url}
        aria-label={url ? t("announce.openLink") : undefined}
      >
        <span className="announce-banner-icon" aria-hidden="true">
          •
        </span>
        <span className="announce-banner-text">{text}</span>
        {url && <span className="announce-banner-arrow">↗</span>}
      </button>
      <button
        type="button"
        className="announce-banner-close"
        onClick={onClose}
        aria-label={t("common.close")}
        title={t("common.close")}
      >
        ×
      </button>
    </div>
  );
}
