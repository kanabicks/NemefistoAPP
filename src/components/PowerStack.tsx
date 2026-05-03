import { useTranslation } from "react-i18next";
import { useVpnStore } from "../stores/vpnStore";
import { PRESET_BUTTON_STYLE } from "../stores/settingsStore";
import { useEffectiveSettings } from "../lib/hooks/useEffectiveSettings";
import { POWER_LABEL_CLS, STATUS_PILL_CLS } from "../lib/constants";
import { PowerIcon } from "./icons";

/**
 * Центральный блок: круглая power-кнопка, status-pill сверху,
 * подпись и текущий режим под кнопкой.
 */
export function PowerStack({ canConnect }: { canConnect: boolean }) {
  const { t } = useTranslation();
  const status = useVpnStore((s) => s.status);
  const mode = useVpnStore((s) => s.mode);
  const socksPort = useVpnStore((s) => s.socksPort);
  const httpPort = useVpnStore((s) => s.httpPort);
  const socksUsername = useVpnStore((s) => s.socksUsername);
  const socksPassword = useVpnStore((s) => s.socksPassword);
  const connect = useVpnStore((s) => s.connect);
  const disconnect = useVpnStore((s) => s.disconnect);
  const { buttonStyle, preset } = useEffectiveSettings();
  const effectiveButtonStyle =
    preset === "none" ? buttonStyle : PRESET_BUTTON_STYLE[preset];

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";

  const onClick = () => {
    if (isBusy) return;
    if (isRunning) void disconnect();
    else void connect();
  };

  return (
    <div className="power-stack">
      <div className={`status-pill ${STATUS_PILL_CLS[status]}`}>
        <span className="dot" />
        <span>{t(`status.pill.${status}`)}</span>
      </div>

      <button
        type="button"
        className={`power-btn power-btn-${effectiveButtonStyle}${isRunning ? " is-running" : ""}`}
        disabled={isRunning ? isBusy : !canConnect}
        onClick={onClick}
        aria-label={isRunning ? t("power.disconnect") : t("power.connect")}
      >
        <PowerIcon />
        <span>
          {isBusy ? "…" : isRunning ? t("power.disconnect") : t("power.connect")}
        </span>
      </button>

      <div style={{ textAlign: "center" }}>
        <div className={`power-label ${POWER_LABEL_CLS[status]}`}>
          {t(`status.label.${status}`)}.
        </div>
        {isRunning && socksPort && (
          <div className="power-detail" style={{ marginTop: 6 }}>
            socks5 127.0.0.1:{socksPort} · http :{httpPort}
          </div>
        )}
        {isRunning && socksUsername && socksPassword && (
          <LanCredentials user={socksUsername} pass={socksPassword} />
        )}
        {!isRunning && (
          <div className="power-detail" style={{ marginTop: 6 }}>
            {t("power.modeLabel", { mode: t(`mode.${mode}`) })}
          </div>
        )}
      </div>
    </div>
  );
}

/** Маленькая плашка с SOCKS5-кредами для LAN-режима + кнопка копирования. */
function LanCredentials({ user, pass }: { user: string; pass: string }) {
  const { t } = useTranslation();
  return (
    <div className="lan-creds" style={{ marginTop: 8 }}>
      <div className="lan-creds-label">{t("power.lanCredsLabel")}</div>
      <button
        type="button"
        className="lan-creds-row"
        onClick={() => {
          void navigator.clipboard.writeText(`${user}:${pass}`);
        }}
        title={t("power.lanCredsCopyTitle")}
      >
        <span className="lan-creds-user">{user}</span>
        <span className="lan-creds-sep">:</span>
        <span className="lan-creds-pass">{pass}</span>
        <span className="lan-creds-copy">⎘</span>
      </button>
    </div>
  );
}
