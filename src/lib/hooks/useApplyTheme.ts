import { useEffect } from "react";
import { useEffectiveSettings } from "./useEffectiveSettings";

/**
 * Синхронизирует effective `theme` или `preset` с атрибутом на <html>.
 * Effective = override-логика из useEffectiveSettings (юзер-настройка
 * перебивает заголовок подписки, иначе используется заголовок).
 *
 * Один из двух взаимоисключающих атрибутов:
 *  - preset !== "none" → `data-preset="..."`, `data-theme` снят;
 *  - иначе             → `data-theme="..."`, `data-preset` снят.
 *
 * CSS переменные в App.css определены в `:root[data-theme="..."]` и
 * `:root[data-preset="..."]` — пресет имеет ту же приоритет-структуру,
 * но переопределяет более широкий набор переменных (палитра + glow + ...).
 */
export function useApplyTheme() {
  const { theme, preset } = useEffectiveSettings();

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
