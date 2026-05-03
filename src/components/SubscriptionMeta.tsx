import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useSubscriptionStore } from "../stores/subscriptionStore";

/**
 * Полоска с метаданными подписки (трафик + срок).
 * Источник — заголовок `subscription-userinfo` подписки.
 */

function formatBytes(b: number, units: string[]): string {
  // units: [бит/байт-эквивалент в порядке: B, KB, MB, GB, TB]
  if (!Number.isFinite(b) || b <= 0) return `0 ${units[0]}`;
  const TB = 1024 ** 4;
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  const KB = 1024;
  if (b >= TB) return (b / TB).toFixed(2) + ` ${units[4]}`;
  if (b >= GB) return (b / GB).toFixed(2) + ` ${units[3]}`;
  if (b >= MB) return (b / MB).toFixed(1) + ` ${units[2]}`;
  if (b >= KB) return (b / KB).toFixed(1) + ` ${units[1]}`;
  return Math.round(b) + ` ${units[0]}`;
}

export function SubscriptionMeta() {
  const { t, i18n } = useTranslation();
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
  const barCls = ratio >= 0.9 ? "is-danger" : ratio >= 0.7 ? "is-warn" : "";

  const units = [
    t("subMeta.units.B"),
    t("subMeta.units.KB"),
    t("subMeta.units.MB"),
    t("subMeta.units.GB"),
    t("subMeta.units.TB"),
  ];

  // Относительное время «N единиц назад» для last-fetched timestamp.
  const formatRelative = (unixMs: number): string => {
    const diff = Date.now() - unixMs;
    if (diff < 0) return t("subMeta.justNow");
    const sec = Math.floor(diff / 1000);
    if (sec < 60) return t("subMeta.justNow");
    const min = Math.floor(sec / 60);
    if (min < 60) return t("subMeta.minutesAgo", { count: min });
    const hour = Math.floor(min / 60);
    if (hour < 24) return t("subMeta.hoursAgo", { count: hour });
    const day = Math.floor(hour / 24);
    if (day < 7) return t("subMeta.daysAgo", { count: day });
    return t("subMeta.longAgo");
  };

  // Срок действия подписки.
  const formatExpiry = (unixSeconds: number): { text: string; warn: boolean } => {
    const now = Date.now() / 1000;
    const diff = unixSeconds - now;
    const day = 86400;

    if (diff < 0) {
      const past = Math.floor(-diff / day);
      return { text: t("subMeta.expiredAgo", { count: past }), warn: true };
    }
    if (diff < day) {
      return { text: t("subMeta.expiresToday"), warn: true };
    }
    const date = new Date(unixSeconds * 1000);
    // Локализованное «до 5 мая» через Intl.DateTimeFormat
    const formatter = new Intl.DateTimeFormat(i18n.language, {
      day: "numeric",
      month: "long",
    });
    return {
      text: t("subMeta.until", { date: formatter.format(date) }),
      warn: diff < 7 * day,
    };
  };

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
              title={t("subMeta.premiumTitle")}
            >
              {t("subMeta.premium")} ↗
            </button>
          )}
        </div>
      )}
      {(hasTraffic || hasExpiry) && (
        <div className="sub-meta-row">
          <span className="sub-meta-traffic">
            {total > 0 ? (
              t("subMeta.trafficUsedOf", {
                used: formatBytes(used, units),
                total: formatBytes(total, units),
              })
            ) : used > 0 ? (
              t("subMeta.trafficUsedUnlim", {
                used: formatBytes(used, units),
              })
            ) : (
              t("subMeta.unlimited")
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
        <div
          className="sub-meta-bar"
          aria-label={t("subMeta.barAria", { percent })}
        >
          <div
            className={`sub-meta-bar-fill ${barCls}`}
            style={{ width: `${percent}%` }}
          />
        </div>
      )}
      {(hasFetchTime || subUrl.trim()) && (
        <div className="sub-meta-fetch">
          {hasFetchTime ? (
            <span>{t("subMeta.updated", { when: formatRelative(lastFetchedAt!) })}</span>
          ) : (
            <span>{t("subMeta.neverUpdated")}</span>
          )}
          {subUrl.trim() && (
            <button
              type="button"
              className={`sub-meta-refresh${subLoading ? " is-loading" : ""}`}
              onClick={() => void fetchSubscription()}
              disabled={subLoading}
              title={t("subMeta.refreshTitle")}
              aria-label={t("subMeta.refreshTitle")}
            >
              ↻
            </button>
          )}
        </div>
      )}
    </div>
  );
}
