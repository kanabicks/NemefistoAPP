import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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
};

type RecoveryReport = {
  kill_switch_cleaned: boolean;
  orphan_resources_cleaned: boolean;
  system_proxy_cleared: boolean;
  errors: string[];
};

export function CrashRecoveryDialog() {
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
    findings.push({ key: "proxy", label: "системный прокси указывает на нас" });
  if (state.proxy_backup_present)
    findings.push({
      key: "backup",
      label: "сохранён backup оригинальных настроек прокси",
    });
  if (state.tun_orphan)
    findings.push({ key: "tun", label: "tun-адаптер `nemefisto-*` в системе" });

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog">
        <div className="recovery-title">обнаружены остатки прошлой сессии</div>
        <div className="recovery-text">
          предыдущий запуск не завершился чисто (краш или принудительная
          остановка). нашли:
        </div>
        <ul className="recovery-list">
          {findings.map((f) => (
            <li key={f.key}>{f.label}</li>
          ))}
        </ul>
        <div className="recovery-text">
          «починить всё» — снимет любые наши wfp-фильтры, удалит
          orphan tun-адаптеры и принудительно очистит системный прокси.
          {state.proxy_backup_present && (
            <>
              {" "}
              «восстановить прокси» — откатит реестр на оригинальные
              значения которые были до подключения.
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
            оставить как есть
          </button>
          {state.proxy_backup_present && (
            <button
              type="button"
              className="btn-ghost"
              onClick={onRestoreBackup}
              disabled={busy}
            >
              восстановить прокси
            </button>
          )}
          <button
            type="button"
            className="btn-primary"
            onClick={onFixAll}
            disabled={busy}
          >
            {busy ? "…" : "починить всё"}
          </button>
        </div>
      </div>
    </div>
  );
}
