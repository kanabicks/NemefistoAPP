import { invoke } from "@tauri-apps/api/core";
import { showToast } from "../stores/toastStore";

/** Бэк-тип из `vpn::leak_test::LeakTestResult` (serde без rename). */
export type LeakTestResult = {
  ip: string | null;
  country_code: string | null;
  country_name: string | null;
  city: string | null;
  dns_resolver: string | null;
  dns_clean: boolean;
  /** 14.D: IPv6 leak. Если не null — наш v6-only запрос прошёл, значит
   *  v6-трафик идёт мимо VPN. */
  ipv6_leak: string | null;
};

/**
 * ISO-3166 alpha-2 → regional-indicator emoji флага.
 * `DE` → 🇩🇪. Возвращает пустую строку если код невалидный.
 */
function flag(code: string | null): string {
  if (!code || code.length !== 2) return "";
  const A = 0x1f1e6;
  const chars = code
    .toUpperCase()
    .split("")
    .map((c) => A + c.charCodeAt(0) - 65);
  // Принимаем только ASCII A-Z (regional indicators).
  if (chars.some((cp) => cp < A || cp > A + 25)) return "";
  return String.fromCodePoint(...chars);
}

/**
 * Запустить leak-test и отобразить тост(ы).
 *
 * - Один success-тост со страной/IP — после успеха.
 * - Если DNS leak обнаружен (резолвер == public IP) → дополнительный
 *   warning-toast.
 * - Если бэк не смог получить IP (нет инета) → error-toast.
 *
 * `socksPort` — наш локальный SOCKS5 inbound (proxy-mode, см. vpnStore).
 * В TUN-режиме передаём null — reqwest пойдёт через system route.
 */
export async function runLeakTest(socksPort: number | null): Promise<void> {
  let result: LeakTestResult;
  try {
    result = await invoke<LeakTestResult>("leak_test", {
      socksPort,
    });
  } catch (e) {
    showToast({
      kind: "error",
      title: "проверка утечек",
      message: `не удалось проверить: ${String(e)}`,
    });
    return;
  }

  if (!result.ip) {
    showToast({
      kind: "error",
      title: "проверка утечек",
      message:
        "не удалось получить публичный ip — оба сервиса (cloudflare, ipwho.is) недоступны",
    });
    return;
  }

  const fl = flag(result.country_code);
  const place = [result.country_name, result.city].filter(Boolean).join(", ");
  const ipLine = result.ip;
  const placeLine = `${fl ? fl + " " : ""}${place || result.country_code || "—"}`;

  showToast({
    kind: "success",
    title: "твой ip сейчас",
    message: `${ipLine}\n${placeLine}`,
    durationMs: 8000,
  });

  // DNS leak: резолвер == public IP → запросы идут через сам клиент
  // мимо VPN. Это явная утечка.
  if (
    result.dns_resolver &&
    !result.dns_clean
  ) {
    showToast({
      kind: "warning",
      title: "dns leak",
      message: `резолвер (${result.dns_resolver}) совпадает с публичным ip — днс не идёт через vpn`,
      durationMs: 12000,
    });
  }

  // 14.D: IPv6 leak — v6-only endpoint ответил, значит трафик IPv6
  // идёт мимо VPN. Туннель покрывает только v4. Лечится включением
  // kill switch (он блокирует весь v6 outbound) или ручным
  // отключением IPv6 на сетевом адаптере.
  if (result.ipv6_leak) {
    showToast({
      kind: "warning",
      title: "ipv6 leak",
      message: `v6-адрес (${result.ipv6_leak}) виден напрямую — туннель v6 не покрывает.\nвключите kill switch или отключите ipv6 в свойствах адаптера`,
      durationMs: 14000,
    });
  }
}
