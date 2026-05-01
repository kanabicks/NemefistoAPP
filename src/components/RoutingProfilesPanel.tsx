import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { showToast } from "../stores/toastStore";

/**
 * 11.G — UI вкладка «маршрутизация» в Settings.
 *
 * Список routing-профилей со state стора (бэкенд-side через `routing_list`).
 * Активный отмечается radio'ом. Для autorouting — индикатор «обновлено
 * N часов назад» + кнопка ручного refresh. Для всех — кнопка delete.
 *
 * Добавление: textarea + select интервала + кнопка. Auto-detect формата
 * payload'а:
 *  - начинается с http(s):// → URL (autorouting если interval > 0)
 *  - иначе → base64 / JSON statиc
 */

type ProfileSource =
  | { kind: "static" }
  | { kind: "autorouting"; url: string; interval_hours: number };

type RoutingProfile = {
  Name?: string;
  GlobalProxy?: string | boolean;
  DirectSites?: string[];
  DirectIp?: string[];
  ProxySites?: string[];
  ProxyIp?: string[];
  BlockSites?: string[];
  BlockIp?: string[];
  Geoipurl?: string;
  Geositeurl?: string;
};

type RoutingEntry = {
  id: string;
  profile: RoutingProfile;
  source: ProfileSource;
  last_fetched_at: number;
};

type Snapshot = {
  entries: RoutingEntry[];
  active_id: string | null;
};

type GeofileStatus = {
  filename: string;
  present: boolean;
  size_bytes: number;
  sha256: string | null;
};

type GeofilesStatus = {
  directory: string;
  geoip: GeofileStatus;
  geosite: GeofileStatus;
};

type GeofilesUpdateReport = {
  geoip_updated: boolean;
  geoip_skipped_unchanged: boolean;
  geosite_updated: boolean;
  geosite_skipped_unchanged: boolean;
  errors: string[];
};

function relativeTime(unixSec: number): string {
  if (unixSec === 0) return "ещё не обновлялся";
  const now = Math.floor(Date.now() / 1000);
  const diff = now - unixSec;
  if (diff < 60) return "только что";
  if (diff < 3600) return `${Math.floor(diff / 60)} мин назад`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} ч назад`;
  return `${Math.floor(diff / 86400)} дн назад`;
}

function ruleCount(p: RoutingProfile): number {
  return (
    (p.DirectSites?.length ?? 0) +
    (p.DirectIp?.length ?? 0) +
    (p.ProxySites?.length ?? 0) +
    (p.ProxyIp?.length ?? 0) +
    (p.BlockSites?.length ?? 0) +
    (p.BlockIp?.length ?? 0)
  );
}

function fmtBytes(n: number): string {
  if (n === 0) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function RoutingProfilesPanel() {
  const [snapshot, setSnapshot] = useState<Snapshot>({
    entries: [],
    active_id: null,
  });
  const [geofiles, setGeofiles] = useState<GeofilesStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [addPayload, setAddPayload] = useState("");
  const [addInterval, setAddInterval] = useState<number>(24);

  const reload = async () => {
    try {
      const s = await invoke<Snapshot>("routing_list");
      setSnapshot(s);
    } catch (e) {
      console.error("[routing] list failed:", e);
    }
    try {
      const g = await invoke<GeofilesStatus>("geofiles_status");
      setGeofiles(g);
    } catch {
      // не критично
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const onAdd = async () => {
    const raw = addPayload.trim();
    if (!raw) return;
    setBusy(true);
    try {
      const isUrl = /^https?:\/\//i.test(raw);
      if (isUrl) {
        await invoke<string>("routing_add_url", {
          url: raw,
          intervalHours: addInterval,
        });
      } else {
        await invoke<string>("routing_add_static", { payload: raw });
      }
      setAddPayload("");
      await reload();
      showToast({
        kind: "success",
        title: "routing-профиль",
        message: "добавлен",
      });
    } catch (e) {
      showToast({
        kind: "error",
        title: "не получилось добавить",
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const onSetActive = async (id: string | null) => {
    setBusy(true);
    try {
      await invoke("routing_set_active", { id });
      await reload();
    } catch (e) {
      showToast({
        kind: "error",
        title: "не удалось активировать",
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const onRefresh = async (id: string) => {
    setBusy(true);
    try {
      await invoke("routing_refresh", { id });
      await reload();
      showToast({
        kind: "success",
        title: "профиль",
        message: "обновлён",
      });
    } catch (e) {
      showToast({
        kind: "error",
        title: "refresh failed",
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const onRemove = async (id: string) => {
    if (!confirm("удалить профиль?")) return;
    setBusy(true);
    try {
      await invoke("routing_remove", { id });
      await reload();
    } catch (e) {
      showToast({
        kind: "error",
        title: "не удалось удалить",
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const onGeofilesRefresh = async () => {
    setBusy(true);
    try {
      const r = await invoke<GeofilesUpdateReport>("geofiles_refresh");
      await reload();
      const updated = [
        r.geoip_updated && "geoip",
        r.geosite_updated && "geosite",
      ].filter(Boolean);
      const skipped = [
        r.geoip_skipped_unchanged && "geoip",
        r.geosite_skipped_unchanged && "geosite",
      ].filter(Boolean);
      if (r.errors.length > 0) {
        showToast({
          kind: "warning",
          title: "geofiles частично",
          message: r.errors.join("; "),
        });
      } else {
        const lines: string[] = [];
        if (updated.length > 0) lines.push(`обновлено: ${updated.join(", ")}`);
        if (skipped.length > 0)
          lines.push(`не изменилось: ${skipped.join(", ")}`);
        showToast({
          kind: "success",
          title: "geofiles",
          message: lines.join("\n") || "нет geofile URLs в активном профиле",
        });
      }
    } catch (e) {
      showToast({
        kind: "error",
        title: "geofiles refresh failed",
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <section className="settings-section">
        <div className="settings-section-title">профили маршрутизации</div>
        <p
          className="hint"
          style={{
            textTransform: "none",
            letterSpacing: 0,
            color: "var(--fg-dim)",
            fontSize: 12,
            lineHeight: 1.5,
            marginBottom: 8,
          }}
        >
          импорт правил split-routing. формат — Marzban-style JSON
          (DirectSites/DirectIp/ProxySites/BlockSites + Geoipurl/Geositeurl).
          один профиль активен — его правила применяются к Xray и Mihomo
          при connect. autorouting обновляется по интервалу из URL-источника.
        </p>

        {snapshot.entries.length === 0 && (
          <div className="settings-row-hint" style={{ marginBottom: 12 }}>
            пусто. добавьте профиль ниже или импортируйте по deep-link
            <br />
            <code>nemefisto://routing/onadd/{"{base64-or-url}"}</code>
          </div>
        )}

        {snapshot.entries.map((e) => {
          const isActive = snapshot.active_id === e.id;
          const isAuto = e.source.kind === "autorouting";
          return (
            <div
              key={e.id}
              className="settings-row"
              style={{
                flexDirection: "column",
                alignItems: "stretch",
                gap: 8,
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  width: "100%",
                }}
              >
                <input
                  type="radio"
                  checked={isActive}
                  onChange={() => onSetActive(e.id)}
                  disabled={busy}
                />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div className="settings-row-label">
                    {e.profile.Name || "без имени"}{" "}
                    <span
                      style={{
                        color: isAuto ? "rgb(120, 220, 200)" : "var(--fg-dim)",
                        fontSize: 11,
                        marginLeft: 6,
                      }}
                    >
                      {isAuto ? "auto" : "static"}
                    </span>
                  </div>
                  <div className="settings-row-hint">
                    {ruleCount(e.profile)} правил
                    {isAuto && (
                      <>
                        {" · обновление каждые "}
                        {(e.source as { interval_hours: number }).interval_hours}
                        ч · {relativeTime(e.last_fetched_at)}
                      </>
                    )}
                  </div>
                </div>
                {isAuto && (
                  <button
                    type="button"
                    className="btn-ghost"
                    onClick={() => onRefresh(e.id)}
                    disabled={busy}
                    title="обновить сейчас"
                  >
                    ↻
                  </button>
                )}
                <button
                  type="button"
                  className="btn-ghost"
                  onClick={() => onRemove(e.id)}
                  disabled={busy}
                  title="удалить"
                >
                  ✕
                </button>
              </div>
            </div>
          );
        })}

        {snapshot.active_id && (
          <div className="settings-row">
            <div>
              <div className="settings-row-label">отключить профиль</div>
              <div className="settings-row-hint">
                деактивировать активный (правила не применяются при connect)
              </div>
            </div>
            <button
              type="button"
              className="btn-ghost"
              onClick={() => onSetActive(null)}
              disabled={busy}
            >
              отключить
            </button>
          </div>
        )}
      </section>

      <section className="settings-section">
        <div className="settings-section-title">добавить профиль</div>
        <textarea
          className="textinput"
          rows={4}
          placeholder={`URL (autorouting) или base64/JSON (статический)\n\nпример URL: https://example.com/routing.json\nпример base64: eyJOYW1lIjoiVGVzdCJ9`}
          value={addPayload}
          onChange={(e) => setAddPayload(e.target.value)}
          style={{
            width: "100%",
            padding: "8px 10px",
            background: "var(--bg-glass)",
            border: "1px solid var(--line)",
            borderRadius: "var(--r-md, 12px)",
            color: "var(--fg)",
            fontSize: 12,
            fontFamily: "var(--font-mono, monospace)",
            resize: "vertical",
            marginBottom: 8,
          }}
        />
        <div
          style={{
            display: "flex",
            gap: 10,
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              fontSize: 12,
              color: "var(--fg-dim)",
            }}
          >
            интервал автообновления:
            <select
              value={addInterval}
              onChange={(e) => setAddInterval(Number(e.target.value))}
              style={{
                background: "var(--bg-glass)",
                border: "1px solid var(--line)",
                borderRadius: 6,
                color: "var(--fg)",
                padding: "3px 6px",
                fontSize: 12,
              }}
            >
              <option value={12}>12 часов</option>
              <option value={24}>24 часа</option>
              <option value={72}>3 дня</option>
              <option value={168}>7 дней</option>
              <option value={8760}>не обновлять</option>
            </select>
            <span style={{ color: "var(--fg-dim)", fontSize: 11 }}>
              (только для URL)
            </span>
          </label>
          <button
            type="button"
            className="btn-primary"
            onClick={onAdd}
            disabled={busy || !addPayload.trim()}
          >
            добавить
          </button>
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-section-title">geofiles (geoip/geosite .dat)</div>
        <p
          className="hint"
          style={{
            textTransform: "none",
            letterSpacing: 0,
            color: "var(--fg-dim)",
            fontSize: 12,
            lineHeight: 1.5,
            marginBottom: 8,
          }}
        >
          v2ray-rules-dat файлы (Loyalsoldier) кешируются локально. URL
          берутся из активного профиля (Geoipurl / Geositeurl). При
          обновлении сначала качается .sha256 — если хэш не изменился,
          .dat пропускается (экономия 5-15 MB каждый раз).
        </p>
        {geofiles && (
          <>
            <div className="settings-row-hint" style={{ marginBottom: 8 }}>
              путь: <code>{geofiles.directory}</code>
            </div>
            {([geofiles.geoip, geofiles.geosite] as const).map((f) => (
              <div className="settings-row" key={f.filename}>
                <div>
                  <div className="settings-row-label">{f.filename}</div>
                  <div className="settings-row-hint">
                    {f.present
                      ? `${fmtBytes(f.size_bytes)}, sha256: ${
                          f.sha256 ? f.sha256.slice(0, 16) + "…" : "—"
                        }`
                      : "не загружен"}
                  </div>
                </div>
              </div>
            ))}
          </>
        )}
        <div className="settings-row">
          <div>
            <div className="settings-row-label">обновить geofiles</div>
            <div className="settings-row-hint">
              скачать свежие .dat если sha256 изменился. URL берутся из
              активного профиля
            </div>
          </div>
          <button
            type="button"
            className="btn-ghost"
            onClick={onGeofilesRefresh}
            disabled={busy || !snapshot.active_id}
          >
            обновить
          </button>
        </div>
      </section>
    </>
  );
}
