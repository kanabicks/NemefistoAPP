import { useVpnStore } from "../stores/vpnStore";
import { PRESET_BUTTON_STYLE } from "../stores/settingsStore";
import { useEffectiveSettings } from "../lib/hooks/useEffectiveSettings";
import { MODE_LABEL, POWER_LABEL, STATUS_PILL } from "../lib/constants";
import { PowerIcon } from "./icons";

/**
 * Центральный блок: круглая power-кнопка, status-pill сверху,
 * подпись и текущий режим под кнопкой.
 *
 * `canConnect` приходит снаружи — он зависит от наличия выбранного
 * сервера, который контролируется в App.
 */
export function PowerStack({ canConnect }: { canConnect: boolean }) {
  const status = useVpnStore((s) => s.status);
  const mode = useVpnStore((s) => s.mode);
  const socksPort = useVpnStore((s) => s.socksPort);
  const httpPort = useVpnStore((s) => s.httpPort);
  const connect = useVpnStore((s) => s.connect);
  const disconnect = useVpnStore((s) => s.disconnect);
  // Effective-значения учитывают override из заголовков подписки.
  const { buttonStyle, preset } = useEffectiveSettings();
  // Если активен пресет — стиль кнопки берётся из его таблицы
  // (см. PRESET_BUTTON_STYLE в settingsStore).
  const effectiveButtonStyle = preset === "none" ? buttonStyle : PRESET_BUTTON_STYLE[preset];

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";
  const pill = STATUS_PILL[status];
  const label = POWER_LABEL[status];

  const onClick = () => {
    if (isBusy) return;
    if (isRunning) void disconnect();
    else void connect();
  };

  return (
    <div className="power-stack">
      <div className={`status-pill ${pill.cls}`}>
        <span className="dot" />
        <span>{pill.label}</span>
      </div>

      <button
        type="button"
        className={`power-btn power-btn-${effectiveButtonStyle}${isRunning ? " is-running" : ""}`}
        disabled={isRunning ? isBusy : !canConnect}
        onClick={onClick}
        aria-label={isRunning ? "отключить" : "подключить"}
      >
        <PowerIcon />
        <span>
          {isBusy ? "…" : isRunning ? "отключить" : "подключить"}
        </span>
      </button>

      <div style={{ textAlign: "center" }}>
        <div className={`power-label ${label.cls}`}>{label.text}.</div>
        {isRunning && socksPort && (
          <div className="power-detail" style={{ marginTop: 6 }}>
            socks5 127.0.0.1:{socksPort} · http :{httpPort}
          </div>
        )}
        {!isRunning && (
          <div className="power-detail" style={{ marginTop: 6 }}>
            режим — {MODE_LABEL[mode]}
          </div>
        )}
      </div>
    </div>
  );
}
