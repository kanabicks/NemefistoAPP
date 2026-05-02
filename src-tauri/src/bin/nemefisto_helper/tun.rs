//! Cleanup orphan TUN-ресурсов (legacy от tun2proxy + sing-box/mihomo
//! built-in TUN, если они упали kill -9 не убрав адаптер).
//!
//! sing-box миграция (0.1.2): tun2proxy spawn (`start()` / `stop()`)
//! выпилен — sing-box и mihomo делают TUN сами через built-in inbound.
//! Этот модуль остался только для:
//!
//! 1. **`cleanup_orphan_resources()`** — на старте helper-сервиса чистит
//!    остатки от упавших сессий: nemefisto-* WinTUN-адаптеры и наши
//!    half-default routes через `198.18.0.1` (старый tun2proxy-префикс).
//!    Идемпотентно. Если TUN-движок (sing-box / mihomo) активен —
//!    проверка не делается (мы не знаем PID активного процесса, но и
//!    не ломаем).
//!
//! 2. **`current_tun_interface_index()`** — стаб для firewall.rs
//!    (kill-switch step A). Раньше возвращал индекс tun2proxy-адаптера;
//!    теперь sing-box/mihomo владеют адаптером сами и helper не знает
//!    их PID. Возвращает `None` — firewall обходится без TUN-specific
//!    allow-фильтра (allow_app для движков покрывает их трафик).

use anyhow::Result;

use super::routing;

/// Префикс имени TUN-адаптера. Используется sing-box (`nemefisto-<pid>`)
/// и mihomo (`nemefisto-mihomo` etc) — общий префикс позволяет одной
/// командой почистить остатки от любого движка.
const TUN_NAME_PREFIX: &str = "nemefisto-";
/// Адрес TUN-интерфейса от tun2proxy-эпохи (0.1.1 и ранее). Сейчас
/// sing-box создаёт TUN с другим IP по умолчанию, но half-routes на этот
/// IP могут остаться от старых сессий — чистим.
const TUN_GATEWAY: &str = "198.18.0.1";
const HALF_LOW_DST: &str = "0.0.0.0";
const HALF_HIGH_DST: &str = "128.0.0.0";
const HALF_MASK: &str = "128.0.0.0";

/// Стаб для firewall.rs (kill-switch). После выпила tun2proxy helper не
/// владеет TUN-адаптером, движки делают это сами. Возвращает `None`,
/// что переводит kill-switch в режим "без TUN-specific allow" — это OK,
/// потому что allow_app для sing-box/mihomo бинарей всё равно даёт
/// нужный outbound.
#[allow(dead_code)]
pub async fn current_tun_interface_index() -> Option<u32> {
    None
}

/// 9.E — Cleanup orphan TUN-ресурсов на старте helper-сервиса.
///
/// После аварийного завершения (kernel panic, kill -9, hardware crash)
/// в системе могут остаться:
///   1. WinTUN-адаптеры с префиксом `nemefisto-` от sing-box / mihomo /
///      legacy tun2proxy.
///   2. Half-default routes (`0.0.0.0/1` и `128.0.0.0/1` через
///      `198.18.0.1`) от legacy tun2proxy-эпохи (sing-box/mihomo
///      используют auto_route и сами чистят при graceful exit).
///
/// Best-effort: каждая операция игнорирует свои ошибки.
pub async fn cleanup_orphan_resources() {
    // 1. nemefisto-* адаптеры. PowerShell wildcard.
    let wildcard = format!("{TUN_NAME_PREFIX}*");
    if let Err(e) = routing::cleanup_orphan_tun(&wildcard).await {
        eprintln!("[helper-tun] cleanup_orphan_tun({wildcard}) → {e}");
    }

    // 2. Legacy half-default routes от tun2proxy-эпохи. Только с нашим
    // nexthop=198.18.0.1 (другой VPN с теми же префиксами не тронем).
    if let Err(e) =
        routing::delete_route_with_nexthop(HALF_LOW_DST, HALF_MASK, TUN_GATEWAY).await
    {
        eprintln!("[helper-tun] cleanup orphan {HALF_LOW_DST}/1 → {e}");
    }
    if let Err(e) =
        routing::delete_route_with_nexthop(HALF_HIGH_DST, HALF_MASK, TUN_GATEWAY).await
    {
        eprintln!("[helper-tun] cleanup orphan {HALF_HIGH_DST}/1 → {e}");
    }
    let _: Result<()> = Ok(());
}
