import type { VpnMode } from "../stores/vpnStore";

/**
 * Segmented control: переключение режима VPN (proxy / tun).
 * Дизейблится пока туннель в running/busy состоянии.
 */
export function ModeSegment({
  mode,
  onChange,
  disabled,
}: {
  mode: VpnMode;
  onChange: (m: VpnMode) => void;
  disabled: boolean;
}) {
  return (
    <div className="mode-seg" style={{ marginTop: 12 }}>
      {(["proxy", "tun"] as VpnMode[]).map((m) => (
        <button
          key={m}
          type="button"
          disabled={disabled}
          onClick={() => onChange(m)}
          className={mode === m ? "is-active" : ""}
        >
          {m === "proxy" ? "системный прокси" : "tun"}
        </button>
      ))}
    </div>
  );
}
