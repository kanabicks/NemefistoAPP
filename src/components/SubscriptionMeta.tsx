import { openUrl } from "@tauri-apps/plugin-opener";
import { useSubscriptionStore } from "../stores/subscriptionStore";

/**
 * Полоска с метаданными подписки (трафик + срок).
 * Источник — заголовок `subscription-userinfo` подписки (стандарт
 * 3x-ui / Marzban / x-ui / sing-box).
 *
 * Показываем только если хотя бы одно поле что-то осмысленное.
 * Если total=0 — безлимит, прогресс-бар не рисуем.
 * Если expireAt отсутствует — кусок «до даты» не рендерим.
 */

const MONTHS_GENITIVE = [
  "января", "февраля", "марта", "апреля", "мая", "июня",
  "июля", "августа", "сентября", "октября", "ноября", "декабря",
];

function formatBytes(b: number): string {
  if (!Number.isFinite(b) || b <= 0) return "0 Б";
  const TB = 1024 ** 4;
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  const KB = 1024;
  if (b >= TB) return (b / TB).toFixed(2) + " ТБ";
  if (b >= GB) return (b / GB).toFixed(2) + " ГБ";
  if (b >= MB) return (b / MB).toFixed(1) + " МБ";
  if (b >= KB) return (b / KB).toFixed(1) + " КБ";
  return Math.round(b) + " Б";
}

/** Относительное время «N единиц назад» для last-fetched timestamp.
 *  Если >7 дней — «давно», если меньше минуты — «только что». 12.B */
function formatRelative(unixMs: number): string {
  const diff = Date.now() - unixMs;
  if (diff < 0) return "только что"; // защита от расхождения часов
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return "только что";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min} мин назад`;
  const hour = Math.floor(min / 60);
  if (hour < 24) return `${hour} ч назад`;
  const day = Math.floor(hour / 24);
  if (day < 7) return `${day} дн назад`;
  return "давно";
}

function formatExpiry(unixSeconds: number): { text: string; warn: boolean } {
  const now = Date.now() / 1000;
  const diff = unixSeconds - now;
  const day = 86400;

  if (diff < 0) {
    const past = Math.floor(-diff / day);
    return { text: `истекла ${past} дн. назад`, warn: true };
  }
  if (diff < day) {
    return { text: "истекает сегодня", warn: true };
  }
  const date = new Date(unixSeconds * 1000);
  const text = `до ${date.getDate()} ${MONTHS_GENITIVE[date.getMonth()]}`;
  // Меньше 7 дней — мягкое предупреждение
  return { text, warn: diff < 7 * day };
}

export function SubscriptionMeta() {
  const meta = useSubscriptionStore((s) => s.meta);
  const lastFetchedAt = useSubscriptionStore((s) => s.lastFetchedAt);
  const subUrl = useSubscriptionStore((s) => s.url);
  const subLoading = useSubscriptionStore((s) => s.loading);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  if (!meta && !lastFetchedAt) return null;

  const used = meta?.used ?? 0;
  const total = meta?.total ?? 0;
  const expireAt = meta?.expireAt ?? null;
  const title = meta?.title ?? null;
  const premiumUrl = meta?.premiumUrl ?? null;
  const hasTraffic = total > 0 || used > 0;
  const hasExpiry = expireAt != null;
  const hasTitle = !!title;
  const hasPremium = !!premiumUrl;
  const hasFetchTime = !!lastFetchedAt;
  if (!hasTraffic && !hasExpiry && !hasTitle && !hasPremium && !hasFetchTime) {
    return null;
  }

  const ratio = total > 0 ? Math.min(1, used / total) : 0;
  const percent = Math.round(ratio * 100);
  // Цвет полосы: зелёный → жёлтый → красный
  const barCls =
    ratio >= 0.9 ? "is-danger" : ratio >= 0.7 ? "is-warn" : "";

  const expiry = hasExpiry ? formatExpiry(expireAt!) : null;

  return (
    <div className="sub-meta">
      {(hasTitle || hasPremium) && (
        <div className="sub-meta-title">
          {hasTitle && <span>{title}</span>}
          {hasPremium && (
            <button
              type="button"
              className="sub-meta-premium"
              onClick={() => premiumUrl && void openUrl(premiumUrl)}
              title="перейти на премиум"
            >
              премиум ↗
            </button>
          )}
        </div>
      )}
      {(hasTraffic || hasExpiry) && (
        <div className="sub-meta-row">
          <span className="sub-meta-traffic">
            {total > 0 ? (
              <>
                использовано {formatBytes(used)} из {formatBytes(total)}
              </>
            ) : used > 0 ? (
              <>использовано {formatBytes(used)} · безлимит</>
            ) : (
              <>безлимит</>
            )}
          </span>
          {expiry && (
            <span
              className={`sub-meta-expiry${expiry.warn ? " is-warn" : ""}`}
            >
              {expiry.text}
            </span>
          )}
        </div>
      )}
      {total > 0 && (
        <div className="sub-meta-bar" aria-label={`использовано ${percent}%`}>
          <div
            className={`sub-meta-bar-fill ${barCls}`}
            style={{ width: `${percent}%` }}
          />
        </div>
      )}
      {(hasFetchTime || subUrl.trim()) && (
        <div className="sub-meta-fetch">
          {hasFetchTime ? (
            <span>обновлено {formatRelative(lastFetchedAt!)}</span>
          ) : (
            <span>не обновлялась</span>
          )}
          {subUrl.trim() && (
            <button
              type="button"
              className={`sub-meta-refresh${subLoading ? " is-loading" : ""}`}
              onClick={() => void fetchSubscription()}
              disabled={subLoading}
              title="обновить подписку"
              aria-label="обновить подписку"
            >
              ↻
            </button>
          )}
        </div>
      )}
    </div>
  );
}
