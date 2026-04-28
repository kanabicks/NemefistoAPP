import { useEffect } from "react";
import { useSettingsStore } from "../../stores/settingsStore";

/**
 * Синхронизирует значение `theme` из settings store с атрибутом
 * `data-theme` на <html>. CSS переменные в App.css переопределяются
 * по селектору `:root[data-theme="light"]`.
 *
 * Применяется в App.tsx один раз — реагирует на смену через store
 * автоматически.
 */
export function useApplyTheme() {
  const theme = useSettingsStore((s) => s.theme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    return () => {
      // На размонтировании вернём дефолт. На практике приложение
      // не размонтируется, но пусть без артефактов в HMR.
      delete document.documentElement.dataset.theme;
    };
  }, [theme]);
}
