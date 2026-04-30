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
  if (!meta) return null;

  const { used, total, expireAt, title, premiumUrl } = meta;
  const hasTraffic = total > 0 || used > 0;
  const hasExpiry = expireAt != null;
  const hasTitle = !!title;
  const hasPremium = !!premiumUrl;
  if (!hasTraffic && !hasExpiry && !hasTitle && !hasPremium) return null;

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
    </div>
  );
}
