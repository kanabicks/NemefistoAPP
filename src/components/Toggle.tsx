/**
 * Универсальный switch-toggle. Управляемый компонент.
 */
export function Toggle({
  on,
  onChange,
  disabled,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={disabled}
      onClick={() => !disabled && onChange(!on)}
      className={`toggle${on ? " is-on" : ""}${disabled ? " is-disabled" : ""}`}
    >
      <span className="toggle-knob" />
    </button>
  );
}
