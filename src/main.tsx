import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Локальные шрифты — bundle-ятся через Vite, никаких внешних запросов
// (Tauri-окно может быть оффлайн).

// Space Grotesk — display (заголовки, имена серверов).
// ВАЖНО: Space Grotesk не поддерживает кириллицу — для русского текста
// браузер падает на следующий шрифт в стэке (см. App.css `--display`,
// там Inter Tight стоит сразу за Space Grotesk).
import "@fontsource/space-grotesk/500.css";
import "@fontsource/space-grotesk/600.css";
import "@fontsource/space-grotesk/700.css";

// Inter Tight — body (русский текст + cyrillic fallback для display).
import "@fontsource/inter-tight/400.css";
import "@fontsource/inter-tight/500.css";
import "@fontsource/inter-tight/cyrillic-400.css";
import "@fontsource/inter-tight/cyrillic-500.css";

// JetBrains Mono — мета, метки, моноширинный текст.
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/cyrillic-400.css";
import "@fontsource/jetbrains-mono/cyrillic-500.css";

// Noto Color Emoji — для флагов стран (regional indicator emoji).
// Без него на Win10 родной Segoe UI Emoji не рендерит флаги, и в именах
// серверов 🇩🇪 / 🇺🇸 / 🇷🇺 показываются как пустые квадраты.
import "@fontsource/noto-color-emoji/400.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
