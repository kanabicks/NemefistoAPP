import { useEffect } from "react";
import "./App.css";
import { useVpnStore, type VpnStatus } from "./stores/vpnStore";

const STATUS_LABELS: Record<VpnStatus, string> = {
  stopped: "Остановлен",
  starting: "Запускается…",
  running: "Запущен (SOCKS5: 127.0.0.1:1080)",
  stopping: "Останавливается…",
  error: "Ошибка",
};

const STATUS_COLORS: Record<VpnStatus, string> = {
  stopped: "text-neutral-400",
  starting: "text-yellow-400",
  running: "text-emerald-400",
  stopping: "text-yellow-400",
  error: "text-red-400",
};

function App() {
  const status = useVpnStore((s) => s.status);
  const errorMessage = useVpnStore((s) => s.errorMessage);
  const start = useVpnStore((s) => s.start);
  const stop = useVpnStore((s) => s.stop);
  const refresh = useVpnStore((s) => s.refresh);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";

  return (
    <main className="min-h-screen flex flex-col items-center justify-center gap-6 bg-neutral-900 text-neutral-100 p-8">
      <h1 className="text-3xl font-semibold">VPN-клиент — Этап 1</h1>
      <p className={`text-base ${STATUS_COLORS[status]}`}>
        {STATUS_LABELS[status]}
      </p>
      {errorMessage && (
        <pre className="text-sm text-red-300 max-w-xl whitespace-pre-wrap break-all">
          {errorMessage}
        </pre>
      )}
      <button
        type="button"
        disabled={isBusy}
        onClick={() => (isRunning ? stop() : start())}
        className={`px-8 py-4 rounded-full font-semibold text-lg transition-colors ${
          isRunning
            ? "bg-red-600 hover:bg-red-500"
            : "bg-emerald-600 hover:bg-emerald-500"
        } disabled:opacity-50 disabled:cursor-not-allowed`}
      >
        {isRunning ? "Остановить Xray" : "Запустить Xray"}
      </button>
      <p className="text-xs text-neutral-500 max-w-md text-center">
        На Этапе 1 Xray слушает локальный SOCKS5 без VPN-outbound — это проверка
        sidecar-механики, не реальный VPN.
      </p>
    </main>
  );
}

export default App;
