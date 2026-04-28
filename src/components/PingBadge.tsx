/**
 * Цветной значок пинга: быстрый/средний/медленный/offline/loading.
 * Цвет управляется CSS-классом, см. App.css `.ping`.
 */

function pingClass(ms: number | null | undefined): string {
  if (ms == null) return "offline";
  if (ms < 80) return "fast";
  if (ms < 200) return "medium";
  return "slow";
}

export function PingBadge({
  ms,
  loading,
}: {
  ms: number | null | undefined;
  loading: boolean;
}) {
  if (loading && ms === undefined) {
    return <span className="ping loading">…</span>;
  }
  if (ms == null) {
    return <span className="ping offline">— ms</span>;
  }
  return <span className={`ping ${pingClass(ms)}`}>{ms} ms</span>;
}
