import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * Crash-recovery диалог (этап 9.D).
 *
 * При старте приложения проверяем `has_proxy_backup()` — если true,
 * значит прошлая сессия не успела вызвать `clear_system_proxy` (краш
 * или `kill`), и в реестре до сих пор стоит наш SOCKS5/HTTP прокси.
 * Без восстановления интернет в стороннем браузере «лежит».
 *
 * Показываем модалку с двумя действиями:
 *  - «Восстановить» → restore_proxy_backup, реестр откатывается на
 *    значения, которые были до нашего connect.
 *  - «Не восстанавливать» → discard_proxy_backup, текущее состояние
 *    реестра остаётся (на случай если пользователь сам уже починил).
 */
export function CrashRecoveryDialog() {
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const has = await invoke<boolean>("has_proxy_backup");
        if (has) setShow(true);
      } catch {
        // не критично — без диалога на старте всё равно работает
      }
    })();
  }, []);

  if (!show) return null;

  const onRestore = async () => {
    setBusy(true);
    setError(null);
    try {
      await invoke("restore_proxy_backup");
      setShow(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onDiscard = async () => {
    setBusy(true);
    try {
      await invoke("discard_proxy_backup");
      setShow(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="recovery-overlay" role="dialog" aria-modal="true">
      <div className="recovery-dialog">
        <div className="recovery-title">обнаружены остатки прошлой сессии</div>
        <div className="recovery-text">
          в реестре Windows остался активным наш системный прокси —
          предыдущий запуск не успел его очистить (вероятно, был краш
          или принудительная остановка). если интернет в браузере
          сейчас не работает — восстанови оригинальные настройки.
        </div>
        {error && <pre className="recovery-error">{error}</pre>}
        <div className="recovery-actions">
          <button
            type="button"
            className="btn-ghost"
            onClick={onDiscard}
            disabled={busy}
          >
            не восстанавливать
          </button>
          <button
            type="button"
            className="btn-primary"
            onClick={onRestore}
            disabled={busy}
          >
            {busy ? "…" : "восстановить"}
          </button>
        </div>
      </div>
    </div>
  );
}
