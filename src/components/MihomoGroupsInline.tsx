import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useVpnStore } from "../stores/vpnStore";
import { useSubscriptionStore } from "../stores/subscriptionStore";
import { useSettingsStore } from "../stores/settingsStore";
import { showToast } from "../stores/toastStore";

/**
 * 8.F (UI v2) — Inline-вид прокси-групп Mihomo на главном экране.
 *
 * Заменяет модальную ProxiesPanel когда выбран mihomo-профиль (полный
 * YAML с `proxy-groups`/`rules`). Отрисовывает FlClash-style сетку
 * карточек: каждая нода (страна) — отдельная карточка, активная
 * подсвечена.
 *
 * **До connect:** данные берутся из `selectedServer.raw.{groups, proxies}`
 * — статика из YAML подписки. Клик по карточке записывает её в
 * `settings.preferredMihomoNodes[group]`. Latency не показываем (mihomo
 * ещё не запущен — нечего опрашивать). При connect `vpnStore` через
 * external-controller применит preferred-выбор сразу после старта.
 *
 * **После connect:** polling `mihomo_proxies` каждые 3 сек. Карточки
 * показывают live latency из `history`. Клик по карточке = мгновенное
 * переключение через external-controller + сохранение как preferred
 * (на следующий перезапуск VPN).
 */

type ProxyInfo = {
  name: string;
  type: string;
  now?: string | null;
  all: string[];
  history: { time: string; delay: number }[];
  udp: boolean;
};

type ProxiesSnapshot = { proxies: Record<string, ProxyInfo> };

const GROUP_TYPES = new Set([
  "Selector",
  "URLTest",
  "Fallback",
  "LoadBalance",
  "Relay",
]);

const YAML_TO_API_GROUP_TYPE: Record<string, string> = {
  select: "Selector",
  "url-test": "URLTest",
  fallback: "Fallback",
  "load-balance": "LoadBalance",
  relay: "Relay",
};

function lastDelay(p: ProxyInfo | undefined): number | null {
  if (!p?.history?.length) return null;
  const v = p.history[p.history.length - 1].delay;
  return v > 0 ? v : null;
}

/** Цвет latency-бэйджа: зелёный <200мс / жёлтый <500 / оранжевый <1000 /
 *  красный либо timeout. Возвращает CSS-класс из App.css. */
function delayClass(d: number | null): string {
  if (d == null) return "delay-none";
  if (d < 200) return "delay-good";
  if (d < 500) return "delay-ok";
  if (d < 1000) return "delay-slow";
  return "delay-bad";
}

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

export function MihomoGroupsInline() {
  const { t } = useTranslation();
  const status = useVpnStore((s) => s.status);
  const selectedIndex = useVpnStore((s) => s.selectedIndex);
  const servers = useSubscriptionStore((s) => s.servers);
  const preferredNodes = useSettingsStore((s) => s.preferredMihomoNodes);
  const setSetting = useSettingsStore((s) => s.set);

  const liveMode = status === "running";

  const typeLabel = (apiType: string): string => {
    const known = ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"];
    if (known.includes(apiType))
      return t(`mihomoGroups.typeLabels.${apiType}`);
    return apiType.toLowerCase();
  };

  const delayLabel = (d: number | null): string =>
    d == null ? "—" : t("mihomoGroups.delayUnit", { value: d });

  const [snap, setSnap] = useState<ProxiesSnapshot | null>(null);
  const [busyTesting, setBusyTesting] = useState<string | null>(null);
  // По умолчанию свёрнуты группы, в которых пользователь руками ничего
  // не выбирает — load-balance/url-test/fallback/relay. Они показывают
  // статус, но карточки нод там работают как «инфо», не как кнопки —
  // нет смысла занимать ими экран. Selector-группы (то что пользователь
  // реально кликает) — раскрыты сразу.
  const [autoCollapsed, setAutoCollapsed] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const refresh = async () => {
    try {
      const data = await invoke<ProxiesSnapshot>("mihomo_proxies");
      setSnap(data);
    } catch {
      // 503 «mihomo не запущен» — нормально между connect-disconnect, молчим
    }
  };

  useEffect(() => {
    if (liveMode) {
      void refresh();
      const t = window.setInterval(() => void refresh(), 3000);
      return () => window.clearInterval(t);
    }
    const entry = selectedIndex !== null ? servers[selectedIndex] : null;
    if (!entry || entry.protocol !== "mihomo-profile") {
      setSnap(null);
      return;
    }
    const raw = entry.raw as
      | {
          groups?: Array<{ name: string; type: string; proxies: string[] }>;
          proxies?: Array<{ name: string; type: string }>;
        }
      | undefined;
    setSnap(buildStaticSnapshot(raw?.groups ?? [], raw?.proxies ?? []));
  }, [liveMode, selectedIndex, servers]);

  const groups = useMemo(() => {
    if (!snap) return [];
    const all = Object.values(snap.proxies).filter(
      (p) => GROUP_TYPES.has(p.type) && p.name !== "GLOBAL"
    );
    // Показываем только «root»-группы — те, на которые не ссылается
    // никакая другая группа как на свой proxy. В типичной подписке
    // вида `ariyvpn (Selector) → [fastest (LoadBalance), latvia, ...]`
    // мы показываем только `ariyvpn`. `fastest` уже виден как карточка
    // внутри `ariyvpn` — дублировать его на верхнем уровне (с тем же
    // содержимым раскрыто) — лишний шум.
    //
    // Fallback: если ВСЕ группы участвуют в членстве (циклы между
    // селекторами), показываем все — лучше чем пустой список.
    const referenced = new Set<string>();
    for (const g of all) {
      for (const member of g.all) referenced.add(member);
    }
    const roots = all.filter((g) => !referenced.has(g.name));
    return (roots.length > 0 ? roots : all).sort((a, b) =>
      a.name.localeCompare(b.name)
    );
  }, [snap]);

  // Один раз после первой загрузки групп — сворачиваем все
  // авто-управляемые root-группы (если они вышли в root, потому что
  // у пользователя нет Selector над ними). Дальше уважаем
  // пользовательские toggle.
  useEffect(() => {
    if (autoCollapsed || groups.length === 0) return;
    const auto = new Set<string>();
    for (const g of groups) {
      if (g.type !== "Selector") auto.add(g.name);
    }
    if (auto.size > 0) setCollapsed(auto);
    setAutoCollapsed(true);
  }, [groups, autoCollapsed]);

  const setPreferred = (group: string, name: string) => {
    setSetting("preferredMihomoNodes", { ...preferredNodes, [group]: name });
  };

  const onCardClick = async (group: string, type: string, name: string) => {
    if (type !== "Selector") {
      // URL-test / fallback / load-balance — нода выбирается автоматом
      // движком. Пользовательский pin не имеет смысла.
      showToast({
        kind: "info",
        title: t("mihomoGroups.toast.autoSelectTitle"),
        message: t("mihomoGroups.toast.autoSelectMessage", {
          type: typeLabel(type),
        }),
        durationMs: 3000,
      });
      return;
    }
    setPreferred(group, name);
    if (!liveMode) {
      showToast({
        kind: "success",
        title: t("mihomoGroups.toast.selectedTitle"),
        message: t("mihomoGroups.toast.selectedPending", { group, name }),
        durationMs: 3000,
      });
      return;
    }
    try {
      await invoke("mihomo_select_proxy", { group, name });
      void refresh();
    } catch (e) {
      showToast({
        kind: "error",
        title: t("mihomoGroups.toast.switchFailedTitle"),
        message: String(e),
      });
    }
  };

  const onTestGroup = async (groupName: string) => {
    if (!liveMode) return;
    setBusyTesting(groupName);
    try {
      await invoke("mihomo_delay_test", { name: groupName });
      void refresh();
    } catch (e) {
      showToast({
        kind: "error",
        title: t("mihomoGroups.toast.testFailedTitle"),
        message: String(e),
      });
    } finally {
      setBusyTesting(null);
    }
  };

  const toggleCollapse = (name: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  if (groups.length === 0) {
    // Подписка распознана как mihomo-profile, но в YAML нет групп —
    // редкий случай. Скрываем секцию полностью, пользователь увидит
    // только обычный server-pill «Профиль Mihomo» в ServerSelector.
    return null;
  }

  return (
    <div className="mihomo-groups">
      {groups.map((g) => {
        const memberInfos = g.all.map((n) => snap!.proxies[n]).filter(Boolean);
        const liveActive = g.now ?? null;
        const preferredName = preferredNodes[g.name];
        const displayActive = liveMode
          ? liveActive
          : preferredName ?? liveActive;
        const isCollapsed = collapsed.has(g.name);
        const isSelector = g.type === "Selector";
        return (
          <section key={g.name} className="mihomo-group">
            <header
              className="mihomo-group-head"
              onClick={() => toggleCollapse(g.name)}
            >
              <div className="mihomo-group-title-block">
                <div className="mihomo-group-title">{g.name}</div>
                <div className="mihomo-group-sub">
                  <span className="mihomo-group-type">{typeLabel(g.type)}</span>
                  {displayActive && (
                    <>
                      <span className="dot-sep">·</span>
                      <span className="mihomo-group-active">
                        {liveMode
                          ? t("mihomoGroups.active")
                          : t("mihomoGroups.selected")}
                        : {displayActive}
                      </span>
                    </>
                  )}
                  <span className="dot-sep">·</span>
                  <span>
                    {t("mihomoGroups.nodeCount", { count: memberInfos.length })}
                  </span>
                  {!isSelector && (
                    <>
                      <span className="dot-sep">·</span>
                      <span style={{ opacity: 0.7 }}>
                        {t("mihomoGroups.auto")}
                      </span>
                    </>
                  )}
                </div>
              </div>
              {liveMode && (
                <button
                  type="button"
                  className="mihomo-test-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    void onTestGroup(g.name);
                  }}
                  disabled={busyTesting === g.name}
                  title={t("mihomoGroups.testTitle")}
                >
                  {busyTesting === g.name ? "…" : t("mihomoGroups.test")}
                </button>
              )}
              <span className="mihomo-group-arrow">
                {isCollapsed ? "▸" : "▾"}
              </span>
            </header>

            {!isCollapsed && (
              <div className="mihomo-grid">
                {memberInfos.map((m) => {
                  const d = lastDelay(m);
                  const isActive = m.name === displayActive;
                  return (
                    <button
                      type="button"
                      key={m.name}
                      className={
                        "mihomo-card" +
                        (isActive ? " is-active" : "") +
                        (isSelector ? "" : " is-readonly")
                      }
                      onClick={() => void onCardClick(g.name, g.type, m.name)}
                      title={
                        isSelector
                          ? liveMode
                            ? t("mihomoGroups.cardTitleLiveSelector")
                            : t("mihomoGroups.cardTitlePreferredSelector")
                          : t("mihomoGroups.cardTitleAuto")
                      }
                    >
                      <div className="mihomo-card-name" title={m.name}>
                        {m.name}
                      </div>
                      <div className="mihomo-card-meta">
                        <span className="mihomo-card-proto">
                          {m.type}
                        </span>
                        {liveMode && (
                          <span className={"mihomo-card-delay " + delayClass(d)}>
                            {delayLabel(d)}
                          </span>
                        )}
                      </div>
                      {isActive && <span className="mihomo-card-check">✓</span>}
                    </button>
                  );
                })}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}
