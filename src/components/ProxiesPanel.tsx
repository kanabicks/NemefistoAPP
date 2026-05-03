import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { showToast } from "../stores/toastStore";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useSettingsStore } from "../stores/settingsStore";

/**
 * 8.F — Панель прокси-групп Mihomo.
 *
 * Два режима:
 *
 * **Live (VPN running)** — данные из mihomo external-controller через
 * `mihomo_proxies`. Видна история latency, можно тыкать ноды для
 * переключения, прогонять `delay_test`. Polling 3 сек.
 *
 * **Static (VPN остановлен)** — синтетический snapshot из
 * `selectedServer.raw.{groups, proxies}`. UI тот же — карточки групп с
 * раскрытием и список нод внутри — но без latency (не из чего брать) и
 * без активного выбора. Это даёт FlClash-style обзор стран ДО connect,
 * чтобы пользователь видел что в подписке вообще лежит.
 */

type ProxyInfo = {
  name: string;
  type: string;
  now?: string | null;
  all: string[];
  history: { time: string; delay: number }[];
  udp: boolean;
};

type ProxiesSnapshot = {
  proxies: Record<string, ProxyInfo>;
};

const GROUP_TYPES = new Set([
  "Selector",
  "URLTest",
  "Fallback",
  "LoadBalance",
  "Relay",
]);

/** YAML-тип группы (kebab-case) → external-controller тип (PascalCase).
 *  Mihomo external-controller отдаёт типы в PascalCase, а в YAML принято
 *  писать `type: select|url-test|...`. Чтобы рендерить group-emoji и
 *  определять «можно ли выбирать ноду» одинаково в live и static режимах,
 *  нормализуем YAML-тип к PascalCase при построении static-snapshot. */
const YAML_TO_API_GROUP_TYPE: Record<string, string> = {
  select: "Selector",
  "url-test": "URLTest",
  fallback: "Fallback",
  "load-balance": "LoadBalance",
  relay: "Relay",
};

/** Тип группы → emoji-бейдж. Подпись берётся через i18n в компоненте. */
const TYPE_EMOJI: Record<string, string> = {
  Selector: "📋",
  URLTest: "🎯",
  Fallback: "🔀",
  LoadBalance: "⚖️",
  Relay: "🔗",
};

function lastDelay(p: ProxyInfo): number | null {
  if (!p.history?.length) return null;
  const v = p.history[p.history.length - 1].delay;
  return v > 0 ? v : null;
}

function delayClass(d: number | null): string {
  if (d == null) return "delay-none";
  if (d < 200) return "delay-good";
  if (d < 500) return "delay-ok";
  if (d < 1000) return "delay-slow";
  return "delay-bad";
}

/** Собрать ProxiesSnapshot из raw.groups/raw.proxies подписки. Используется
 *  до connect — UI рендерит то же самое, но без latency-history и без
 *  активной ноды (мы пока не подняли mihomo, поэтому реальный «now» не
 *  знаем — берём первую из списка group.proxies как «по умолчанию»). */
function buildStaticSnapshot(
  rawGroups: Array<{ name: string; type: string; proxies: string[] }>,
  rawProxies: Array<{ name: string; type: string }>
): ProxiesSnapshot {
  const out: Record<string, ProxyInfo> = {};
  for (const p of rawProxies) {
    out[p.name] = {
      name: p.name,
      type: p.type,
      now: null,
      all: [],
      history: [],
      udp: false,
    };
  }
  for (const g of rawGroups) {
    const apiType = YAML_TO_API_GROUP_TYPE[g.type] ?? "Selector";
    out[g.name] = {
      name: g.name,
      type: apiType,
      now: g.proxies[0] ?? null,
      all: g.proxies,
      history: [],
      udp: false,
    };
  }
  return { proxies: out };
}

export function ProxiesPanel({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const status = useVpnStore((s) => s.status);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const servers = useSubscriptionStore((s) => s.servers);
  const preferredNodes = useSettingsStore((s) => s.preferredMihomoNodes);
  const setSetting = useSettingsStore((s) => s.set);

  const isRunning = status === "running";
  const liveMode = isRunning;

  const typeInfo = (
    apiType: string
  ): { emoji: string; label: string } => {
    const known = ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"];
    if (known.includes(apiType)) {
      return {
        emoji: TYPE_EMOJI[apiType] ?? "📦",
        label: t(`proxiesPanel.typeLabels.${apiType}`),
      };
    }
    return { emoji: "📦", label: apiType.toLowerCase() };
  };

  const delayLabel = (d: number | null): string =>
    d == null ? "—" : t("proxiesPanel.delayUnit", { value: d });

  /** Запомнить выбор пользователя на будущее: ключ — имя группы,
   *  значение — имя ноды. Используется в vpnStore.connect после старта
   *  mihomo чтобы автоматически переключиться на предпочитаемую ноду. */
  const setPreferred = (group: string, name: string) => {
    setSetting("preferredMihomoNodes", { ...preferredNodes, [group]: name });
  };

  const [snap, setSnap] = useState<ProxiesSnapshot | null>(null);
  const [loading, setLoading] = useState(liveMode);
  const [error, setError] = useState<string | null>(null);
  const [busyTesting, setBusyTesting] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const refresh = async () => {
    if (!liveMode) return;
    try {
      const data = await invoke<ProxiesSnapshot>("mihomo_proxies");
      setSnap(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  // Live: периодический pull external-controller. Static: построить
  // snapshot один раз из raw подписки.
  useEffect(() => {
    if (liveMode) {
      void refresh();
      const t = window.setInterval(() => void refresh(), 3000);
      return () => window.clearInterval(t);
    }
    const entry = selectedIndex !== null ? servers[selectedIndex] : null;
    if (!entry || entry.protocol !== "mihomo-profile") {
      setSnap(null);
      setLoading(false);
      return;
    }
    const raw = entry.raw as
      | { groups?: Array<{ name: string; type: string; proxies: string[] }>; proxies?: Array<{ name: string; type: string }> }
      | undefined;
    setSnap(buildStaticSnapshot(raw?.groups ?? [], raw?.proxies ?? []));
    setLoading(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveMode, selectedIndex, servers]);

  // Только группы (исключаем встроенные DIRECT/REJECT/GLOBAL и
  // прокси-ноды — они показываются вложенно в группах).
  const groups = useMemo(() => {
    if (!snap) return [];
    return Object.values(snap.proxies)
      .filter((p) => GROUP_TYPES.has(p.type))
      .filter((p) => p.name !== "GLOBAL")
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [snap]);

  // По умолчанию раскрываем первую группу — пользователю обычно нужно
  // сразу увидеть её содержимое (особенно когда группа одна, как в
  // типовых подписках с одним select-роутером).
  useEffect(() => {
    if (groups.length > 0 && expanded.size === 0) {
      setExpanded(new Set([groups[0].name]));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups.length]);

  const toggleGroup = (name: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const selectNode = async (group: string, name: string) => {
    // В обоих режимах сохраняем как предпочитаемую — это и для подсказки
    // UI «активная: …» в static, и для авто-применения при следующем
    // connect (vpnStore.connect → applyPreferredMihomoNodes).
    setPreferred(group, name);

    if (!liveMode) {
      showToast({
        kind: "success",
        title: t("proxiesPanel.toast.selectedTitle"),
        message: t("proxiesPanel.toast.selectedPending", { group, name }),
        durationMs: 3500,
      });
      return;
    }
    try {
      await invoke("mihomo_select_proxy", { group, name });
      showToast({
        kind: "success",
        title: t("proxiesPanel.toast.switchedTitle"),
        message: t("proxiesPanel.toast.switchedMessage", { group, name }),
      });
      void refresh();
    } catch (e) {
      showToast({
        kind: "error",
        title: t("proxiesPanel.toast.switchFailedTitle"),
        message: String(e),
      });
    }
  };

  const testNode = async (name: string) => {
    if (!liveMode) return;
    setBusyTesting(name);
    try {
      const ms = await invoke<number | null>("mihomo_delay_test", { name });
      showToast({
        kind: ms == null ? "warning" : "success",
        title: name,
        message:
          ms == null
            ? t("proxiesPanel.timeout")
            : t("proxiesPanel.delayMs", { value: ms }),
      });
      void refresh();
    } catch (e) {
      showToast({
        kind: "error",
        title: t("proxiesPanel.toast.testFailedTitle"),
        message: String(e),
      });
    } finally {
      setBusyTesting(null);
    }
  };

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div
        className="recovery-dialog"
        style={{ maxWidth: 480, maxHeight: "80vh", display: "flex", flexDirection: "column" }}
      >
        <div className="recovery-title">
          {t("proxiesPanel.title")}
          {liveMode ? "" : t("proxiesPanel.titleBeforeConnect")}
        </div>

        {loading && (
          <div className="recovery-text">{t("proxiesPanel.loading")}</div>
        )}
        {error && <pre className="recovery-error">{error}</pre>}

        {!loading && !error && groups.length === 0 && (
          <div className="recovery-text">
            {t("proxiesPanel.emptyState")}
          </div>
        )}

        {!liveMode && groups.length > 0 && (
          <div
            className="recovery-text"
            style={{ fontSize: 11, color: "var(--fg-dim)" }}
          >
            {t("proxiesPanel.staticHint")}
          </div>
        )}

        <div
          style={{
            flex: 1,
            overflowY: "auto",
            display: "flex",
            flexDirection: "column",
            gap: 8,
            margin: "12px 0",
          }}
        >
          {groups.map((g) => {
            const info = typeInfo(g.type);
            const isExpanded = expanded.has(g.name);
            const memberInfos = g.all
              .map((n) => snap!.proxies[n])
              .filter(Boolean);
            const activeName = g.now ?? null;

            return (
              <section
                key={g.name}
                style={{
                  border: "1px solid var(--border)",
                  borderRadius: 6,
                  padding: 10,
                }}
              >
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 10,
                    cursor: "pointer",
                  }}
                  onClick={() => toggleGroup(g.name)}
                >
                  <span style={{ fontSize: 16 }}>{info.emoji}</span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      style={{
                        fontFamily: "var(--font-mono, monospace)",
                        fontSize: 13,
                        whiteSpace: "nowrap",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                      }}
                      title={g.name}
                    >
                      {g.name}
                    </div>
                    <div
                      style={{
                        fontSize: 11,
                        color: "var(--fg-dim)",
                      }}
                    >
                      {info.label}
                      {(() => {
                        const active = liveMode
                          ? activeName
                          : preferredNodes[g.name] ?? activeName;
                        if (!active) return "";
                        const status = liveMode
                          ? t("proxiesPanel.active")
                          : t("proxiesPanel.selected");
                        return ` · ${status}: ${active}`;
                      })()}
                      {t("proxiesPanel.nodeCount", {
                        count: memberInfos.length,
                      })}
                    </div>
                  </div>
                  {liveMode && (
                    <button
                      type="button"
                      className="btn-ghost"
                      style={{ fontSize: 11, padding: "4px 8px" }}
                      onClick={(e) => {
                        e.stopPropagation();
                        void testNode(g.name);
                      }}
                      disabled={busyTesting === g.name}
                    >
                      {busyTesting === g.name ? "…" : t("proxiesPanel.test")}
                    </button>
                  )}
                  <span style={{ color: "var(--fg-dim)" }}>
                    {isExpanded ? "▾" : "▸"}
                  </span>
                </div>

                {isExpanded && memberInfos.length > 0 && (
                  <ul
                    style={{
                      listStyle: "none",
                      padding: 0,
                      margin: "8px 0 0",
                      display: "flex",
                      flexDirection: "column",
                      gap: 2,
                    }}
                  >
                    {memberInfos.map((m) => {
                      const d = lastDelay(m);
                      // Live: активная — то что external-controller вернул
                      // в `now`. Static: что пользователь сохранил
                      // в preferredMihomoNodes (или первая в group по
                      // дефолту). Selector — единственный тип где выбор
                      // имеет смысл; URL-test/Fallback решают сами.
                      const preferredName =
                        preferredNodes[g.name] ?? activeName;
                      const isActive = liveMode
                        ? m.name === activeName
                        : m.name === preferredName;
                      const canSelect = g.type === "Selector";
                      return (
                        <li
                          key={m.name}
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 8,
                            padding: "4px 6px",
                            borderRadius: 4,
                            cursor: canSelect ? "pointer" : "default",
                            background: isActive
                              ? "rgba(100,180,255,0.08)"
                              : "transparent",
                            fontFamily: "var(--font-mono, monospace)",
                            fontSize: 12,
                          }}
                          onClick={() =>
                            canSelect && !isActive && void selectNode(g.name, m.name)
                          }
                          title={
                            canSelect
                              ? liveMode
                                ? t("proxiesPanel.nodeTitleLiveSelector")
                                : t("proxiesPanel.nodeTitlePreferredSelector")
                              : t("proxiesPanel.nodeTitleAuto")
                          }
                        >
                          <span
                            style={{
                              flex: 1,
                              minWidth: 0,
                              whiteSpace: "nowrap",
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              color: isActive ? "var(--fg)" : "var(--fg-dim)",
                            }}
                          >
                            {isActive ? "● " : "  "}
                            {m.name}
                            {!liveMode && m.type !== "unknown" && (
                              <span
                                style={{
                                  marginLeft: 6,
                                  fontSize: 10,
                                  color: "var(--fg-dim)",
                                  opacity: 0.6,
                                }}
                              >
                                {m.type}
                              </span>
                            )}
                          </span>
                          {liveMode && (
                            <span
                              className={delayClass(d)}
                              style={{
                                fontSize: 11,
                                minWidth: 50,
                                textAlign: "right",
                              }}
                            >
                              {delayLabel(d)}
                            </span>
                          )}
                          {liveMode && (
                            <button
                              type="button"
                              className="btn-ghost"
                              style={{ fontSize: 10, padding: "2px 6px" }}
                              onClick={(e) => {
                                e.stopPropagation();
                                void testNode(m.name);
                              }}
                              disabled={busyTesting === m.name}
                            >
                              {busyTesting === m.name ? "…" : t("proxiesPanel.test")}
                            </button>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>
            );
          })}
        </div>

        <div className="recovery-actions">
          {liveMode && (
            <button type="button" className="btn-ghost" onClick={() => void refresh()}>
              {t("proxiesPanel.refresh")}
            </button>
          )}
          <button type="button" className="btn-primary" onClick={onClose}>
            {t("proxiesPanel.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
