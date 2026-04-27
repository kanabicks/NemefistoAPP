import "./App.css";
import { useExampleStore } from "./stores/exampleStore";

function App() {
  const counter = useExampleStore((s) => s.counter);
  const increment = useExampleStore((s) => s.increment);
  const reset = useExampleStore((s) => s.reset);

  return (
    <main className="min-h-screen flex flex-col items-center justify-center gap-6 bg-neutral-900 text-neutral-100">
      <h1 className="text-3xl font-semibold">VPN-клиент — Этап 0</h1>
      <p className="text-neutral-400">
        Базовый шаблон Tauri 2 + React + TS + Tailwind + Zustand.
      </p>
      <div className="flex flex-col items-center gap-3">
        <span className="text-5xl font-mono tabular-nums">{counter}</span>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={increment}
            className="px-4 py-2 rounded-md bg-emerald-600 hover:bg-emerald-500 transition-colors font-medium"
          >
            +1
          </button>
          <button
            type="button"
            onClick={reset}
            className="px-4 py-2 rounded-md bg-neutral-700 hover:bg-neutral-600 transition-colors font-medium"
          >
            сброс
          </button>
        </div>
      </div>
    </main>
  );
}

export default App;
