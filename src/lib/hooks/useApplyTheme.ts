import { useEffect } from "react";
import { useSettingsStore } from "../../stores/settingsStore";

/**
 * Синхронизирует значение `theme` или `preset` из settings store с
 * атрибутом на <html>. Один из двух взаимоисключающих:
 *  - preset !== "none" → `data-preset="..."`, `data-theme` снят;
 *  - иначе             → `data-theme="..."`, `data-preset` снят.
 *
 * CSS переменные в App.css определены в `:root[data-theme="..."]` и
 * `:root[data-preset="..."]` — пресет имеет ту же приоритет-структуру,
 * но переопределяет более широкий набор переменных (палитра + glow + ...).
 */
export function useApplyTheme() {
  const theme = useSettingsStore((s) => s.theme);
  const preset = useSettingsStore((s) => s.preset);

  useEffect(() => {
    const root = document.documentElement;
    if (preset !== "none") {
      root.dataset.preset = preset;
      delete root.dataset.theme;
    } else {
      root.dataset.theme = theme;
      delete root.dataset.preset;
    }
    return () => {
      delete root.dataset.theme;
      delete root.dataset.preset;
    };
  }, [theme, preset]);
}
