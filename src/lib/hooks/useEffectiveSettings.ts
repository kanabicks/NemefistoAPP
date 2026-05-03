import {
  useSettingsStore,
  type Background,
  type ButtonStyle,
  type Preset,
  type Theme,
} from "../../stores/settingsStore";
import { useSubscriptionStore } from "../../stores/subscriptionStore";
import { useSystemTheme } from "./useSystemTheme";

/**
 * Override-логика 8.C для server-driven UX:
 *
 *   effective[key] = userTouched[key]
 *     ? userOverride[key]
 *     : subscriptionMeta[key] ?? userOverride[key]
 *
 * Если пользователь явно менял настройку (флаг `*Touched=true`),
 * используется его значение; иначе — из заголовка подписки;
 * иначе — текущее значение settings store (= дефолт).
 *
 * Hook разлогинивается на изменения settings и meta — компонент
 * перерендерится при любом из них.
 */

const THEME_VALUES = [
  "system",
  "dark",
  "light",
  "midnight",
  "sunset",
  "sand",
] as const;
const BACKGROUND_VALUES = ["crystal", "tunnel", "globe", "particles"] as const;
const BUTTON_STYLE_VALUES = ["glass", "flat", "neon", "metallic"] as const;
const PRESET_VALUES = [
  "none",
  "fluent",
  "cupertino",
  "vice",
  "arcade",
  "glacier",
] as const;

/** Сужает строку из meta до union-литерала, если она в whitelist. */
function pick<T extends string>(
  value: string | null | undefined,
  allowed: readonly T[]
): T | null {
  if (!value) return null;
  return (allowed as readonly string[]).includes(value) ? (value as T) : null;
}

export type EffectiveSettings = {
  theme: Theme;
  background: Background;
  buttonStyle: ButtonStyle;
  preset: Preset;
  /** Поля, реально пришедшие из подписки (для UI-бейджей «из подписки»). */
  fromSubscription: {
    theme: boolean;
    background: boolean;
    buttonStyle: boolean;
    preset: boolean;
  };
};

export function useEffectiveSettings(): EffectiveSettings {
  const theme = useSettingsStore((s) => s.theme);
  const systemTheme = useSystemTheme();
  const background = useSettingsStore((s) => s.background);
  const buttonStyle = useSettingsStore((s) => s.buttonStyle);
  const preset = useSettingsStore((s) => s.preset);
  const themeTouched = useSettingsStore((s) => s.themeTouched);
  const backgroundTouched = useSettingsStore((s) => s.backgroundTouched);
  const buttonStyleTouched = useSettingsStore((s) => s.buttonStyleTouched);
  const presetTouched = useSettingsStore((s) => s.presetTouched);
  const meta = useSubscriptionStore((s) => s.meta);

  const metaTheme = pick(meta?.theme, THEME_VALUES);
  const metaBackground = pick(meta?.background, BACKGROUND_VALUES);
  const metaButtonStyle = pick(meta?.buttonStyle, BUTTON_STYLE_VALUES);
  const metaPreset = pick(meta?.preset, PRESET_VALUES);

  const useMetaTheme = !themeTouched && metaTheme !== null;
  const useMetaBackground = !backgroundTouched && metaBackground !== null;
  const useMetaButtonStyle = !buttonStyleTouched && metaButtonStyle !== null;
  const useMetaPreset = !presetTouched && metaPreset !== null;

  // Резолвим "system" в реальное dark/light по prefers-color-scheme.
  // Делаем ПОСЛЕ override-логики — даже если в подписке прислан
  // `theme: "system"`, мы тоже подставим текущее системное значение.
  const rawTheme = useMetaTheme ? (metaTheme as Theme) : theme;
  const resolvedTheme: Theme =
    rawTheme === "system" ? (systemTheme as Theme) : rawTheme;

  return {
    theme: resolvedTheme,
    background: useMetaBackground ? (metaBackground as Background) : background,
    buttonStyle: useMetaButtonStyle
      ? (metaButtonStyle as ButtonStyle)
      : buttonStyle,
    preset: useMetaPreset ? (metaPreset as Preset) : preset,
    fromSubscription: {
      theme: useMetaTheme,
      background: useMetaBackground,
      buttonStyle: useMetaButtonStyle,
      preset: useMetaPreset,
    },
  };
}
