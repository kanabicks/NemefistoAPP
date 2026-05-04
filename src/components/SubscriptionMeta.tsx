import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  useSubscriptionStore,
  type ProxyEntry,
  type Subscription,
} from "../stores/subscriptionStore";
import { useVpnStore } from "../stores/vpnStore";
import { MihomoGroupsInline } from "./MihomoGroupsInline";

/**
 * Список карточек подписок (0.3.0 multi-subscription). Каждая карточка
 * показывает свою подписку: трафик, срок, ⋯ меню (engine override +
 * удалить эту). Кнопка «+ добавить» — глобальная, в Header, диспатчит
 * `nemefisto:open-add-subscription` event'ом, который ловит этот компонент
 * и открывает inline-форму поверх списка карточек.
 */
export function SubscriptionMeta() {
  const subscriptions = useSubscriptionStore((s) => s.subscriptions);
  const addSubscription = useSubscriptionStore((s) => s.addSubscription);
  const legacyMeta = useSubscriptionStore((s) => s.meta);
  const legacyLastFetchedAt = useSubscriptionStore((s) => s.lastFetchedAt);
  const legacyUrl = useSubscriptionStore((s) => s.url);

  // Глобальная add-form (открывается из Header → +). Висит над списком,
  // не привязана к конкретной карточке. Закрытие после успешного Add.
  const [globalAddOpen, setGlobalAddOpen] = useState(false);
  const [globalAddUrl, setGlobalAddUrl] = useState("");
  const [globalAdding, setGlobalAdding] = useState(false);
  useEffect(() => {
    const onOpen = () => setGlobalAddOpen(true);
    window.addEventListener("nemefisto:open-add-subscription", onOpen);
    return () =>
      window.removeEventListener("nemefisto:open-add-subscription", onOpen);
  }, []);

  const renderAddForm = () =>
    globalAddOpen ? (
      <div className="sub-meta-global-add">
        <input
          type="url"
          className="sub-meta-add-input"
          value={globalAddUrl}
          onChange={(e) => setGlobalAddUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setGlobalAddOpen(false);
              setGlobalAddUrl("");
            }
          }}
          placeholder="https://sub.example.com/..."
          autoFocus
        />
        <button
          type="button"
          className="sub-meta-add-confirm"
          disabled={globalAdding || !globalAddUrl.trim()}
          onClick={() => {
            if (!globalAddUrl.trim()) return;
            setGlobalAdding(true);
            void addSubscription(globalAddUrl.trim()).finally(() => {
              setGlobalAdding(false);
              setGlobalAddOpen(false);
              setGlobalAddUrl("");
            });
          }}
        >
          {globalAdding ? "…" : "+"}
        </button>
        <button
          type="button"
          className="sub-meta-add-cancel"
          onClick={() => {
            setGlobalAddOpen(false);
            setGlobalAddUrl("");
          }}
        >
          ×
        </button>
      </div>
    ) : null;

  // Backward compat для самого первого запуска: subscriptions пуст, но
  // legacy meta/url пришли. Тогда рендерим один «виртуальный» card. После
  // первого fetchSubscription из Welcome будет миграция → subscriptions[0].
  if (subscriptions.length === 0) {
    if (!legacyMeta && !legacyLastFetchedAt) return renderAddForm();
    if (!legacyUrl.trim()) return renderAddForm();
    return (
      <>
        {renderAddForm()}
        <SubscriptionCard
          sub={{
            id: "__legacy__",
            url: legacyUrl,
            hwid: "",
            meta: legacyMeta,
            lastFetchedAt: legacyLastFetchedAt,
            loading: false,
            error: null,
            engineOverride: null,
            servers: [],
            pings: [],
          }}
          isLegacy
        />
      </>
    );
  }

  return (
    <>
      {renderAddForm()}
      {subscriptions.map((sub) => (
        <SubscriptionCard key={sub.id} sub={sub} />
      ))}
    </>
  );
}

function formatBytes(b: number, units: string[]): string {
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

type SubscriptionCardProps = {
  sub: Subscription;
  /** true когда subscriptions[] пуст и рендерим из legacy-state до миграции. */
  isLegacy?: boolean;
};

function SubscriptionCard({ sub, isLegacy = false }: SubscriptionCardProps) {
  const { t, i18n } = useTranslation();
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  const fetchSubscriptionById = useSubscriptionStore(
    (s) => s.fetchSubscriptionById
  );
  const deleteSubscription = useSubscriptionStore((s) => s.deleteSubscription);
  const removeSubscription = useSubscriptionStore((s) => s.removeSubscription);
  const setEngineOverride = useSubscriptionStore((s) => s.setEngineOverride);
  const setPrimaryId = useSubscriptionStore((s) => s.setPrimaryId);
  const subscriptions = useSubscriptionStore((s) => s.subscriptions);
  const primaryId = useSubscriptionStore((s) => s.primaryId);
  // VPN store hooks для server selection.
  const vpnStatus = useVpnStore((s) => s.status);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const selectServer = useVpnStore((s) => s.selectServer);
  // Selected server name (для подсветки selected row внутри expand'а).
  const selectedName = useSubscriptionStore((st) =>
    selectedIndex !== null ? st.servers[selectedIndex]?.name ?? null : null
  );

  const isPrimary = sub.id === primaryId;
  const subBusy = sub.loading;

  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [popupPos, setPopupPos] = useState<{ top: number; right: number } | null>(
    null
  );
  const menuBtnRef = useRef<HTMLButtonElement | null>(null);

  // Закрытие меню по клику вне или Esc.
  useEffect(() => {
    if (!menuOpen) return;
    const onClick = (e: MouseEvent) => {
      const target = e.target as HTMLElement;
      // Игнорируем клики внутри popup'а (он рендерится в portal,
      // не внутри menuBtnRef-родителя). Используем data-attribute.
      if (target.closest("[data-sub-menu-popup]")) return;
      if (menuBtnRef.current && menuBtnRef.current.contains(target)) return;
      setMenuOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  // Position popup'а относительно ⋯ кнопки. Через layout-effect, чтобы
  // померить ДО первого рендера popup'а (no flicker).
  useLayoutEffect(() => {
    if (!menuOpen || !menuBtnRef.current) return;
    const r = menuBtnRef.current.getBoundingClientRect();
    setPopupPos({
      top: r.bottom + 6,
      right: window.innerWidth - r.right,
    });
  }, [menuOpen]);

  // 0.3.0 Etap 6: при первом expand карточки автоматически загружаем
  // её серверы, если не загружены. Не дёргаем для legacy-card (id =
  // __legacy__) — там используется обычный fetchSubscription.
  useEffect(() => {
    if (!expanded || isLegacy) return;
    if (sub.servers.length > 0 || subBusy) return;
    if (isPrimary) {
      void fetchSubscription();
    } else {
      void fetchSubscriptionById(sub.id);
    }
  }, [expanded]); // eslint-disable-line react-hooks/exhaustive-deps

  /** Выбрать сервер из этой подписки. Если подписка не primary —
   *  переключаем primary на неё (Rust state получит её servers через
   *  fetchSubscription, чтобы connect взял правильный entry по index). */
  const handleSelectServer = async (entry: ProxyEntry) => {
    if (vpnStatus === "starting" || vpnStatus === "stopping") return;
    if (!isPrimary && !isLegacy) {
      setPrimaryId(sub.id);
      // После swap fetchSubscription использует новые URL/HWID/UA через
      // legacy state.url, который setPrimaryId уже синхронизировал.
      await fetchSubscription();
    }
    // Найти индекс в свежем legacy state.servers (после возможного swap).
    const flat = useSubscriptionStore.getState().servers;
    const idx = flat.findIndex(
      (s) =>
        s.name === entry.name &&
        (s.subscriptionId === sub.id ||
          (isLegacy && s.subscriptionId === undefined))
    );
    if (idx >= 0) selectServer(idx);
  };

  const meta = sub.meta;
  const lastFetchedAt = sub.lastFetchedAt;

  const used = meta?.used ?? 0;
  const total = meta?.total ?? 0;
  const expireAt = meta?.expireAt ?? null;
  const title = meta?.title ?? null;
  const premiumUrl = meta?.premiumUrl ?? null;
  const hasTraffic = total > 0 || used > 0;
  const hasExpiry = expireAt != null;
  // Fallback: если у подписки нет title из meta (только что добавлена,
  // ещё не fetch'ена) — показываем host из URL.
  const fallbackTitle = (() => {
    try {
      return new URL(sub.url).host;
    } catch {
      return sub.url;
    }
  })();
  const displayTitle = title || fallbackTitle;
  const hasPremium = !!premiumUrl;
  const hasFetchTime = !!lastFetchedAt;
  // 0.3.0: рендерим карточку даже если meta пуста — иначе свежедобавленная
  // подписка (без fetch'а) невидима, и юзер думает что «не добавилось».
  // Без url'а — нечего показывать, пропускаем.
  if (!sub.url.trim()) return null;

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

  const formatExpiry = (
    unixSeconds: number
  ): { text: string; warn: boolean } => {
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

  const onConfirmDelete = () => {
    setConfirmOpen(false);
    if (isLegacy) {
      void deleteSubscription();
    } else {
      void removeSubscription(sub.id);
    }
  };

  const renderEngineRadio = (
    opt: "auto" | "sing-box" | "mihomo",
    label: string
  ) => {
    const current = sub.engineOverride;
    const isActive =
      (opt === "auto" && current === null) || opt === current;
    return (
      <button
        key={opt}
        type="button"
        role="menuitemradio"
        aria-checked={isActive}
        className={`sub-meta-menu-item${isActive ? " is-active" : ""}`}
        onClick={() => {
          if (isLegacy) return; // legacy одиночный — engine только в Settings
          setEngineOverride(sub.id, opt === "auto" ? null : opt);
          setMenuOpen(false);
          // Smart-refetch: новый engine → новый UA → подписка отдаст
          // другой формат (xray-json для sing-box, clash YAML для mihomo).
          // Без refetch остаются старые servers с прежним engine_compat.
          if (sub.url.trim()) {
            if (isPrimary) {
              void fetchSubscription();
            } else {
              void fetchSubscriptionById(sub.id);
            }
          }
        }}
        disabled={isLegacy}
        title={
          opt === "auto" ? t("subMeta.menuEngineAutoHint") : undefined
        }
      >
        <span className="sub-meta-menu-radio">{isActive ? "●" : "○"}</span>
        <span>{label}</span>
      </button>
    );
  };

  return (
    <div className={`sub-meta${isPrimary ? " is-active" : ""}${expanded ? " is-expanded" : ""}`}>
      <div className="sub-meta-menu">
        <button
          type="button"
          className="sub-meta-expand-btn"
          aria-label={
            expanded
              ? t("subMeta.collapseAria")
              : t("subMeta.expandAria")
          }
          aria-expanded={expanded}
          onClick={() => setExpanded((v) => !v)}
        >
          <svg
            viewBox="0 0 24 24"
            width="14"
            height="14"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{
              transform: expanded ? "rotate(180deg)" : "rotate(0deg)",
              transition: "transform 0.18s ease",
            }}
          >
            <polyline points="6 9 12 15 18 9" />
          </svg>
        </button>
        <button
          ref={menuBtnRef}
          type="button"
          className="sub-meta-menu-btn"
          aria-label={t("subMeta.menuAria")}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((v) => !v)}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor">
            <circle cx="5" cy="12" r="1.6" />
            <circle cx="12" cy="12" r="1.6" />
            <circle cx="19" cy="12" r="1.6" />
          </svg>
        </button>
      </div>
      {/* Popup — Portal в body, чтобы backdrop-filter родителя не создавал
          containing block и popup не обрезался границами карточки. */}
      {menuOpen &&
        popupPos &&
        createPortal(
          <div
            className="sub-meta-menu-popup"
            data-sub-menu-popup
            role="menu"
            style={{
              position: "fixed",
              top: popupPos.top,
              right: popupPos.right,
            }}
          >
            <div className="sub-meta-menu-section-title">
              {t("subMeta.menuEngineTitle")}
            </div>
            {renderEngineRadio("auto", t("subMeta.menuEngineAuto"))}
            {renderEngineRadio("sing-box", "sing-box")}
            {renderEngineRadio("mihomo", "Mihomo")}
            <div className="sub-meta-menu-divider" />
            <button
              type="button"
              role="menuitem"
              className="sub-meta-menu-item is-danger"
              onClick={() => {
                setMenuOpen(false);
                setConfirmOpen(true);
              }}
            >
              {t("subMeta.deleteAction")}
            </button>
          </div>,
          document.body
        )}
      <div className="sub-meta-title">
        <span>{displayTitle}</span>
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
      {(hasFetchTime || sub.url.trim()) && (
        <div className="sub-meta-fetch">
          {hasFetchTime ? (
            <span>
              {t("subMeta.updated", { when: formatRelative(lastFetchedAt!) })}
            </span>
          ) : (
            <span>{t("subMeta.neverUpdated")}</span>
          )}
          {sub.url.trim() && (
            <button
              type="button"
              className={`sub-meta-refresh${subBusy ? " is-loading" : ""}`}
              onClick={() => {
                if (subBusy) return;
                if (isPrimary || isLegacy) {
                  void fetchSubscription();
                } else {
                  void fetchSubscriptionById(sub.id);
                }
              }}
              disabled={subBusy}
              title={t("subMeta.refreshTitle")}
              aria-label={t("subMeta.refreshTitle")}
            >
              ↻
            </button>
          )}
        </div>
      )}
      {/* 0.3.0 Etap 6: server list внутри карточки. Появляется при expand.
          Серверы tag'ятся subscriptionId — каждая карточка показывает
          только свои. Click → handleSelectServer (с auto-swap primary).
          Особый случай: mihomo-passthrough (одна синтетическая запись
          «Профиль Mihomo») — рендерим MihomoGroupsInline вместо row. */}
      {expanded && (
        <div className="sub-meta-servers">
          {sub.loading && sub.servers.length === 0 && (
            <div className="sub-meta-servers-empty">
              {t("subMeta.serversLoading")}
            </div>
          )}
          {!sub.loading && sub.servers.length === 0 && (
            <div className="sub-meta-servers-empty">
              {sub.error
                ? `${t("common.error")}: ${sub.error}`
                : t("subMeta.serversEmpty")}
            </div>
          )}
          {sub.servers.length === 1 &&
          sub.servers[0].protocol === "mihomo-profile" ? (
            // Mihomo-passthrough: показываем proxy-groups вместо одной
            // синтетической записи. MihomoGroupsInline получает entry
            // как prop — читает группы из sub.servers[0].raw напрямую,
            // не зависит от global state. Click по карточке-ноде в
            // non-primary sub автоматически делает swap primary через
            // onActivate callback.
            <MihomoGroupsInline
              entry={sub.servers[0]}
              showSelection={isPrimary}
              onActivate={async () => {
                if (!isPrimary && !isLegacy) {
                  setPrimaryId(sub.id);
                  await fetchSubscription();
                }
                // Critical: selectServer на синтетический mihomo-profile
                // entry (он первый и единственный в state.servers после
                // swap primary). Без этого vpnStore.selectedIndex остаётся
                // null → canConnect=false → кнопка «Подключить» disabled.
                const flat = useSubscriptionStore.getState().servers;
                const idx = flat.findIndex(
                  (s) => s.protocol === "mihomo-profile"
                );
                if (idx >= 0) selectServer(idx);
              }}
            />
          ) : (
            sub.servers.map((entry, i) => {
              const ping = sub.pings[i];
              const isSelected =
                isPrimary && selectedName === entry.name;
              return (
                <button
                  key={`${entry.name}-${i}`}
                  type="button"
                  className={`sub-meta-server-row${isSelected ? " is-selected" : ""}`}
                  onClick={() => void handleSelectServer(entry)}
                  disabled={vpnStatus === "starting" || vpnStatus === "stopping"}
                >
                  <span className="sub-meta-server-name">{entry.name}</span>
                  <span className="sub-meta-server-ping">
                    {ping !== null && ping !== undefined
                      ? `${ping} ms`
                      : "—"}
                  </span>
                </button>
              );
            })
          )}
        </div>
      )}
      {/* Старая per-card inline-форма «добавить» удалена — теперь
          add-button глобальный (в Header), форма рендерится в
          SubscriptionMeta-обёртке поверх списка карточек. */}
      {/* Confirm-модалка через Portal в document.body. */}
      {confirmOpen &&
        createPortal(
          <div
            className="sub-meta-confirm-overlay"
            role="dialog"
            aria-modal="true"
            onClick={() => setConfirmOpen(false)}
          >
            <div
              className="sub-meta-confirm"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="sub-meta-confirm-title">
                {t("subMeta.deleteConfirmTitle")}
              </div>
              <div className="sub-meta-confirm-message">
                {t("subMeta.deleteConfirmMessage")}
              </div>
              <div className="sub-meta-confirm-actions">
                <button
                  type="button"
                  className="sub-meta-confirm-cancel"
                  onClick={() => setConfirmOpen(false)}
                >
                  {t("subMeta.deleteConfirmNo")}
                </button>
                <button
                  type="button"
                  className="sub-meta-confirm-delete"
                  onClick={onConfirmDelete}
                >
                  {t("subMeta.deleteConfirmYes")}
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
      {/* Подавить не-используемые subscriptions reactivity (для force re-render
          когда добавляется новая подписка через addSubscription). */}
      <span style={{ display: "none" }}>{subscriptions.length}</span>
    </div>
  );
}
