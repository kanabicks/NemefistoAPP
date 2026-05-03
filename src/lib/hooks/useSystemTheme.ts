import { useEffect, useState } from "react";

/**
 * Подписка на системную тему через `prefers-color-scheme`.
 * Возвращает `"dark"` или `"light"` и обновляется при смене темы ОС
 * (Windows / macOS / Linux дают этот сигнал через CSS media query).
 *
 * Используется в `useEffectiveSettings` когда `settings.theme === "system"`
 * — резолвим в реальное значение dark/light для применения CSS-переменных.
 */
export function useSystemTheme(): "dark" | "light" {
  const [theme, setTheme] = useState<"dark" | "light">(() => {
    if (typeof window === "undefined" || !window.matchMedia) return "dark";
    return window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  });

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => {
      setTheme(e.matches ? "dark" : "light");
    };
    // Современные браузеры: addEventListener; Safari < 14 — addListener
    if (typeof mq.addEventListener === "function") {
      mq.addEventListener("change", onChange);
      return () => mq.removeEventListener("change", onChange);
    }
    return undefined;
  }, []);

  return theme;
}
