import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

/**
 * 14.E — Расширенный crash-recovery диалог.
 *
 * При старте app вызываем `get_recovery_state` — он проверяет четыре
 * сигнала остатков от прошлой сессии:
 *  - `proxy_orphan` — реестр HKCU указывает на наш SOCKS5/HTTP прокси,
 *    но xray не запущен → браузер «сломан»;
 *  - `proxy_backup_present` — есть `proxy_backup.json` от прошлого
 *    `set_system_proxy`, можно сделать восстановление оригинала;
 *  - `tun_orphan` — в системе остался адаптер `nemefisto-*`;
 *  - `was_crashed` — общий флаг что хоть что-то найдено.
 *
 * Если все четыре false — диалог не показываем.
 *
 * Кнопки:
 *  - **«починить всё»** → `recover_network` (kill_switch_force_cleanup +
 *    orphan_cleanup + force_clear_system_proxy);
 *  - **«восстановить прокси»** → `restore_proxy_backup` (только если
 *    `proxy_backup_present`); откатывает реестр на оригинальные значения;
 *  - **«оставить как есть»** → `discard_proxy_backup` если backup был,
 *    иначе просто закрыть диалог.
 */
type RecoveryState = {
  was_crashed: boolean;
  proxy_orphan: boolean;
  proxy_backup_present: boolean;
  tun_orphan: boolean;
  /** 14.E: остатки WFP-фильтров от прошлой сессии (best-effort через
   *  helper). Если helper не отвечает — false (не пугаем зря). */
  orphan_wfp_filters: boolean;
};

type RecoveryReport = {
  kill_switch_cleaned: boolean;
  orphan_resources_cleaned: boolean;
  system_proxy_cleared: boolean;
  errors: string[];
};

export function CrashRecoveryDialog() {
  const { t } = useTranslation();
  const [state, setState] = useState<RecoveryState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const s = await invoke<RecoveryState>("get_recovery_state");
        if (s.was_crashed) setState(s);
      } catch {
        // не критично
      }
    })();
  }, []);

  if (!state) return null;

  const close = () => setState(null);

  const onFixAll = async () => {
    setBusy(true);
    setError(null);
    try {
      const report = await invoke<RecoveryReport>("recover_network");
      // Если был backup и пользователь жмёт «починить» — backup
      // удаляем, состояние реестра уже свежее (force_clear отработал).
      if (state.proxy_backup_present) {
        await invoke("discard_proxy_backup").catch(() => {});
      }
      if (report.errors.length > 0) {
        setError(report.errors.join("; "));
        setBusy(false);
        return;
      }
      close();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const onRestoreBackup = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("restore_proxy_backup");
      close();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const onLeaveAsIs = async () => {
    setBusy(true);
    try {
      // backup можно discardнуть, остальные orphan'ы пользователь
      // оставил сознательно — пусть лежат.
      if (state.proxy_backup_present) {
        await invoke("discard_proxy_backup").catch(() => {});
      }
      close();
    } finally {
      setBusy(false);
    }
  };

  // Считаем сколько orphan'ов нашли — для текста в шапке.
  const findings: { key: string; label: string }[] = [];
  if (state.proxy_orphan)
    findings.push({ key: "proxy", label: t("modal.crashRecovery.findings.proxy") });
  if (state.proxy_backup_present)
    findings.push({
      key: "backup",
      label: t("modal.crashRecovery.findings.backup"),
    });
  if (state.tun_orphan)
    findings.push({ key: "tun", label: t("modal.crashRecovery.findings.tun") });
  if (state.orphan_wfp_filters)
    findings.push({
      key: "wfp",
      label: t("modal.crashRecovery.findings.wfp"),
    });

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog">
        <div className="recovery-title">{t("modal.crashRecovery.title")}</div>
        <div className="recovery-text">
          {t("modal.crashRecovery.intro")}
        </div>
        <ul className="recovery-list">
          {findings.map((f) => (
            <li key={f.key}>{f.label}</li>
          ))}
        </ul>
        <div className="recovery-text">
          {t("modal.crashRecovery.explainBase")}
          {state.proxy_backup_present && (
            <>
              {" "}
              {t("modal.crashRecovery.explainRestoreProxy")}
            </>
          )}
        </div>
        {error && <pre className="recovery-error">{error}</pre>}
        <div className="recovery-actions">
          <button
            type="button"
            className="btn-ghost"
            onClick={onLeaveAsIs}
            disabled={busy}
          >
            {t("modal.crashRecovery.leaveAsIs")}
          </button>
          {state.proxy_backup_present && (
            <button
              type="button"
              className="btn-ghost"
              onClick={onRestoreBackup}
              disabled={busy}
            >
              {t("modal.crashRecovery.restoreProxy")}
            </button>
          )}
          <button
            type="button"
            className="btn-primary"
            onClick={onFixAll}
            disabled={busy}
          >
            {busy ? "…" : t("modal.crashRecovery.fixAll")}
          </button>
        </div>
      </div>
    </div>
  );
}
