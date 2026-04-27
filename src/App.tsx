import { useEffect } from "react";
import "./App.css";
import { useVpnStore, type VpnStatus } from "./stores/vpnStore";
import { useSubscriptionStore } from "./stores/subscriptionStore";

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

const PROTOCOL_BADGE: Record<string, string> = {
  vless: "bg-violet-700",
  vmess: "bg-blue-700",
  trojan: "bg-amber-700",
  ss: "bg-teal-700",
};

function App() {
  const status = useVpnStore((s) => s.status);
  const errorMessage = useVpnStore((s) => s.errorMessage);
  const start = useVpnStore((s) => s.start);
  const stop = useVpnStore((s) => s.stop);
  const refresh = useVpnStore((s) => s.refresh);

  const servers = useSubscriptionStore((s) => s.servers);
  const subLoading = useSubscriptionStore((s) => s.loading);
  const subError = useSubscriptionStore((s) => s.error);
  const subUrl = useSubscriptionStore((s) => s.url);
  const setSubUrl = useSubscriptionStore((s) => s.setUrl);
  const fetchSubscription = useSubscriptionStore((s) => s.fetchSubscription);
  const loadCached = useSubscriptionStore((s) => s.loadCached);

  useEffect(() => {
    refresh();
    loadCached();
  }, [refresh, loadCached]);

  const isBusy = status === "starting" || status === "stopping";
  const isRunning = status === "running";

  return (
    <main className="min-h-screen flex flex-col items-center gap-8 bg-neutral-900 text-neutral-100 p-8">
      {/* ── Статус и кнопка ── */}
      <section className="flex flex-col items-center gap-4 mt-8">
        <h1 className="text-3xl font-semibold">NemefistoVPN</h1>
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
          {isRunning ? "Отключить" : "Подключить"}
        </button>
      </section>

      {/* ── Подписка ── */}
      <section className="w-full max-w-xl flex flex-col gap-3">
        <h2 className="text-lg font-medium text-neutral-300">Подписка</h2>
        <div className="flex gap-2">
          <input
            type="url"
            value={subUrl}
            onChange={(e) => setSubUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && fetchSubscription()}
            placeholder="https://sub.example.com/…"
            className="flex-1 rounded-lg bg-neutral-800 border border-neutral-700 px-3 py-2 text-sm placeholder-neutral-500 focus:outline-none focus:border-neutral-500"
          />
          <button
            type="button"
            disabled={subLoading || !subUrl.trim()}
            onClick={() => fetchSubscription()}
            className="px-4 py-2 rounded-lg bg-neutral-700 hover:bg-neutral-600 text-sm font-medium disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {subLoading ? "Загрузка…" : "Обновить"}
          </button>
        </div>
        {subError && (
          <pre className="text-xs text-red-300 whitespace-pre-wrap break-all">
            {subError}
          </pre>
        )}
      </section>

      {/* ── Список серверов ── */}
      {servers.length > 0 && (
        <section className="w-full max-w-xl flex flex-col gap-2">
          <h2 className="text-lg font-medium text-neutral-300">
            Серверы ({servers.length})
          </h2>
          <ul className="flex flex-col gap-1">
            {servers.map((s, i) => (
              <li
                key={i}
                className="flex items-center gap-3 rounded-lg bg-neutral-800 px-4 py-2 text-sm"
              >
                <span
                  className={`shrink-0 rounded px-1.5 py-0.5 text-xs font-mono uppercase ${
                    PROTOCOL_BADGE[s.protocol] ?? "bg-neutral-600"
                  }`}
                >
                  {s.protocol}
                </span>
                <span className="flex-1 truncate">{s.name}</span>
                <span className="shrink-0 text-neutral-500">
                  {s.server}:{s.port}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

export default App;
