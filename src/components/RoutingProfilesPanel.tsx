import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
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

function useRelativeTime(): (unixSec: number) => string {
  const { t } = useTranslation();
  return (unixSec: number) => {
    if (unixSec === 0) return t("routingProfiles.relativeTime.neverUpdated");
    const now = Math.floor(Date.now() / 1000);
    const diff = now - unixSec;
    if (diff < 60) return t("routingProfiles.relativeTime.justNow");
    if (diff < 3600)
      return t("routingProfiles.relativeTime.minutesAgo", {
        count: Math.floor(diff / 60),
      });
    if (diff < 86400)
      return t("routingProfiles.relativeTime.hoursAgo", {
        count: Math.floor(diff / 3600),
      });
    return t("routingProfiles.relativeTime.daysAgo", {
      count: Math.floor(diff / 86400),
    });
  };
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
  const { t } = useTranslation();
  const relativeTime = useRelativeTime();
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
        title: t("routingProfiles.toast.addedTitle"),
        message: t("routingProfiles.toast.addedMessage"),
      });
    } catch (e) {
      showToast({
        kind: "error",
        title: t("routingProfiles.toast.addFailedTitle"),
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
        title: t("routingProfiles.toast.activateFailedTitle"),
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
        title: t("routingProfiles.toast.refreshedTitle"),
        message: t("routingProfiles.toast.refreshedMessage"),
      });
    } catch (e) {
      showToast({
        kind: "error",
        title: t("routingProfiles.toast.refreshFailedTitle"),
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const onRemove = async (id: string) => {
    if (!confirm(t("routingProfiles.confirmRemove"))) return;
    setBusy(true);
    try {
      await invoke("routing_remove", { id });
      await reload();
    } catch (e) {
      showToast({
        kind: "error",
        title: t("routingProfiles.toast.removeFailedTitle"),
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
      ].filter(Boolean) as string[];
      const skipped = [
        r.geoip_skipped_unchanged && "geoip",
        r.geosite_skipped_unchanged && "geosite",
      ].filter(Boolean) as string[];
      if (r.errors.length > 0) {
        showToast({
          kind: "warning",
          title: t("routingProfiles.toast.geofilesPartialTitle"),
          message: r.errors.join("; "),
        });
      } else {
        const lines: string[] = [];
        if (updated.length > 0)
          lines.push(
            t("routingProfiles.toast.geofilesUpdated", {
              items: updated.join(", "),
            })
          );
        if (skipped.length > 0)
          lines.push(
            t("routingProfiles.toast.geofilesUnchanged", {
              items: skipped.join(", "),
            })
          );
        showToast({
          kind: "success",
          title: t("routingProfiles.toast.geofilesTitle"),
          message:
            lines.join("\n") || t("routingProfiles.toast.geofilesNoUrls"),
        });
      }
    } catch (e) {
      showToast({
        kind: "error",
        title: t("routingProfiles.toast.geofilesFailedTitle"),
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <section className="settings-section">
        <div className="settings-section-title">
          {t("routingProfiles.title")}
        </div>
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
          {t("routingProfiles.intro")}
        </p>

        {snapshot.entries.length === 0 && (
          <div className="settings-row-hint" style={{ marginBottom: 12 }}>
            {t("routingProfiles.emptyHint")}
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
                    {e.profile.Name || t("routingProfiles.noName")}{" "}
                    <span
                      style={{
                        color: isAuto ? "rgb(120, 220, 200)" : "var(--fg-dim)",
                        fontSize: 11,
                        marginLeft: 6,
                      }}
                    >
                      {isAuto
                        ? t("routingProfiles.kindAuto")
                        : t("routingProfiles.kindStatic")}
                    </span>
                  </div>
                  <div className="settings-row-hint">
                    {t("routingProfiles.ruleCount", {
                      count: ruleCount(e.profile),
                    })}
                    {isAuto &&
                      t("routingProfiles.updateEvery", {
                        hours: (e.source as { interval_hours: number })
                          .interval_hours,
                        when: relativeTime(e.last_fetched_at),
                      })}
                  </div>
                </div>
                {isAuto && (
                  <button
                    type="button"
                    className="btn-ghost"
                    onClick={() => onRefresh(e.id)}
                    disabled={busy}
                    title={t("routingProfiles.refreshNow")}
                  >
                    ↻
                  </button>
                )}
                <button
                  type="button"
                  className="btn-ghost"
                  onClick={() => onRemove(e.id)}
                  disabled={busy}
                  title={t("routingProfiles.deleteTitle")}
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
              <div className="settings-row-label">
                {t("routingProfiles.deactivate.label")}
              </div>
              <div className="settings-row-hint">
                {t("routingProfiles.deactivate.hint")}
              </div>
            </div>
            <button
              type="button"
              className="btn-ghost"
              onClick={() => onSetActive(null)}
              disabled={busy}
            >
              {t("routingProfiles.deactivate.button")}
            </button>
          </div>
        )}
      </section>

      <section className="settings-section">
        <div className="settings-section-title">
          {t("routingProfiles.addTitle")}
        </div>
        <textarea
          className="textinput"
          rows={4}
          placeholder={t("routingProfiles.addPlaceholder")}
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
            {t("routingProfiles.intervalLabel")}
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
              <option value={12}>{t("routingProfiles.intervalOptions.12h")}</option>
              <option value={24}>{t("routingProfiles.intervalOptions.24h")}</option>
              <option value={72}>{t("routingProfiles.intervalOptions.3d")}</option>
              <option value={168}>{t("routingProfiles.intervalOptions.7d")}</option>
              <option value={8760}>{t("routingProfiles.intervalOptions.never")}</option>
            </select>
            <span style={{ color: "var(--fg-dim)", fontSize: 11 }}>
              {t("routingProfiles.intervalUrlOnly")}
            </span>
          </label>
          <button
            type="button"
            className="btn-primary"
            onClick={onAdd}
            disabled={busy || !addPayload.trim()}
          >
            {t("common.add")}
          </button>
        </div>
      </section>

      <section className="settings-section">
        <div className="settings-section-title">
          {t("routingProfiles.geofiles.title")}
        </div>
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
          {t("routingProfiles.geofiles.intro")}
        </p>
        {geofiles && (
          <>
            <div className="settings-row-hint" style={{ marginBottom: 8 }}>
              {t("routingProfiles.geofiles.pathLabel")}{" "}
              <code>{geofiles.directory}</code>
            </div>
            {([geofiles.geoip, geofiles.geosite] as const).map((f) => (
              <div className="settings-row" key={f.filename}>
                <div>
                  <div className="settings-row-label">{f.filename}</div>
                  <div className="settings-row-hint">
                    {f.present
                      ? t("routingProfiles.geofiles.fileSize", {
                          size: fmtBytes(f.size_bytes),
                          hash: f.sha256 ? f.sha256.slice(0, 16) + "…" : "—",
                        })
                      : t("routingProfiles.geofiles.fileMissing")}
                  </div>
                </div>
              </div>
            ))}
          </>
        )}
        <div className="settings-row">
          <div>
            <div className="settings-row-label">
              {t("routingProfiles.geofiles.refreshLabel")}
            </div>
            <div className="settings-row-hint">
              {t("routingProfiles.geofiles.refreshHint")}
            </div>
          </div>
          <button
            type="button"
            className="btn-ghost"
            onClick={onGeofilesRefresh}
            disabled={busy || !snapshot.active_id}
          >
            {t("routingProfiles.geofiles.refreshButton")}
          </button>
        </div>
      </section>
    </>
  );
}
