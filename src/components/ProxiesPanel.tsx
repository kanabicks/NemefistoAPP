import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { showToast } from "../stores/toastStore";

/**
 * 8.F — Панель прокси-групп Mihomo.
 *
 * Показывается когда подключён mihomo-profile (full YAML с
 * `proxy-groups`). Через external-controller API:
 *  - читает все группы и ноды (`mihomo_proxies`),
 *  - даёт переключать ноду в `select`-группе кликом
 *    (`mihomo_select_proxy`),
 *  - запускает тест задержки (`mihomo_delay_test`).
 *
 * Polling: автообновление каждые 3 секунды пока панель открыта.
 *
 * Цветовая схема latency:
 *  - зелёный    < 200ms
 *  - жёлтый     200..500
 *  - оранжевый  500..1000
 *  - красный    >= 1000ms или 0 (timeout)
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

/** Тип группы → emoji-бейдж + русская подпись. */
const TYPE_LABELS: Record<string, { emoji: string; label: string }> = {
  Selector: { emoji: "📋", label: "выбор" },
  URLTest: { emoji: "🎯", label: "url-тест" },
  Fallback: { emoji: "🔀", label: "fallback" },
  LoadBalance: { emoji: "⚖️", label: "load-balance" },
  Relay: { emoji: "🔗", label: "relay" },
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

function delayLabel(d: number | null): string {
  return d == null ? "—" : `${d} мс`;
}

export function ProxiesPanel({ onClose }: { onClose: () => void }) {
  const [snap, setSnap] = useState<ProxiesSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyTesting, setBusyTesting] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const refresh = async () => {
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

  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(t);
  }, []);

  // Только группы (исключаем встроенные DIRECT/REJECT/GLOBAL и
  // прокси-ноды — они показываются вложенно в группах).
  const groups = useMemo(() => {
    if (!snap) return [];
    return Object.values(snap.proxies)
      .filter((p) => GROUP_TYPES.has(p.type))
      .filter((p) => p.name !== "GLOBAL")
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [snap]);

  const toggleGroup = (name: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const selectNode = async (group: string, name: string) => {
    try {
      await invoke("mihomo_select_proxy", { group, name });
      showToast({
        kind: "success",
        title: "переключено",
        message: `${group} → ${name}`,
      });
      void refresh();
    } catch (e) {
      showToast({
        kind: "error",
        title: "не удалось переключить",
        message: String(e),
      });
    }
  };

  const testNode = async (name: string) => {
    setBusyTesting(name);
    try {
      const ms = await invoke<number | null>("mihomo_delay_test", { name });
      showToast({
        kind: ms == null ? "warning" : "success",
        title: name,
        message: ms == null ? "timeout" : `${ms} мс`,
      });
      void refresh();
    } catch (e) {
      showToast({
        kind: "error",
        title: "тест не удался",
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
        <div className="recovery-title">прокси-группы (mihomo)</div>

        {loading && <div className="recovery-text">загрузка…</div>}
        {error && (
          <pre className="recovery-error">{error}</pre>
        )}

        {!loading && !error && groups.length === 0 && (
          <div className="recovery-text">
            в текущем профиле нет групп. подключитесь к mihomo-профилю с
            подписки чтобы увидеть proxy-groups.
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
            const info = TYPE_LABELS[g.type] ?? {
              emoji: "📦",
              label: g.type.toLowerCase(),
            };
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
                      {activeName ? ` · активна: ${activeName}` : ""}
                      {` · ${memberInfos.length} нод`}
                    </div>
                  </div>
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
                    {busyTesting === g.name ? "…" : "тест"}
                  </button>
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
                      const isActive = m.name === activeName;
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
                          title={canSelect ? "выбрать эту ноду" : "управляется автоматически"}
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
                          </span>
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
                            {busyTesting === m.name ? "…" : "тест"}
                          </button>
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
          <button type="button" className="btn-ghost" onClick={() => void refresh()}>
            обновить
          </button>
          <button type="button" className="btn-primary" onClick={onClose}>
            закрыть
          </button>
        </div>
      </div>
    </div>
  );
}
