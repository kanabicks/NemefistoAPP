//! Cleanup orphan TUN-ресурсов (legacy от tun2proxy + sing-box/mihomo
//! built-in TUN, если они упали kill -9 не убрав адаптер) и поиск
//! активного нашего TUN-адаптера для kill-switch'а.
//!
//! sing-box миграция (0.1.2): tun2proxy spawn (`start()` / `stop()`)
//! выпилен — sing-box и mihomo делают TUN сами через built-in inbound.
//! Этот модуль остался только для:
//!
//! 1. **`cleanup_orphan_resources()`** — на старте helper-сервиса чистит
//!    остатки от упавших сессий: nemefisto-* WinTUN-адаптеры и наши
//!    half-default routes через `198.18.0.1` (старый tun2proxy-префикс).
//!
//! 2. **`current_tun_interface_index()`** — поиск активного TUN-адаптера
//!    нашего движка для kill-switch'а (13.D step A). Без TUN allow-фильтра
//!    user-трафик идущий через TUN блокируется WFP block-all'ом — allow_app
//!    покрывает только sing-box/mihomo.exe (их собственные шифрованные
//!    пакеты к серверу, НЕ proxied user-трафик).
//!
//!    Детект многоступенчатый, потому что Wintun-адаптеры разных форков
//!    отличаются по `Description`: WireGuard ставит `"Wintun Userspace
//!    Tunnel"`, sing-box — `"sing-box"`, mihomo — `"Mihomo"`. Делаем:
//!
//!    1. **Alias prefix `nemefisto-`** — наш default-формат (`nemefisto-<pid>`).
//!       Нужно матчить и для sing-box, и для mihomo если в YAML стоит
//!       наш override.
//!    2. **Description ∈ {sing-box, Mihomo, wintun, WireGuard}** —
//!       case-insensitive substring. Покрывает 12.E маскированные имена
//!       (`wlan99` etc) когда alias не помогает.
//!    3. **IP-address в `198.18.0.0/15`** — финальная проверка, что
//!       адаптер ИМЕННО НАШ (RFC 2544 benchmark range, оба движка тут).
//!       Дисамбигуирует если у юзера параллельно WireGuard или другой VPN.
//!
//!    Если один кандидат — возвращаем сразу. Если несколько —
//!    кросс-референс с IP-таблицей. Если ноль — вернёт `None` и kill-
//!    switch встанет без TUN-allow → user-трафик блокируется (kill-switch
//!    делает свою работу, просто слишком жёстко).

use std::ffi::OsString;
use std::mem;
use std::net::Ipv4Addr;
use std::os::windows::ffi::OsStringExt;
use std::time::Duration;

use anyhow::Result;
use windows_sys::Win32::Foundation::NO_ERROR;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, GetUnicastIpAddressTable, MIB_IF_TABLE2,
    MIB_UNICASTIPADDRESS_TABLE,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};

use super::helper_log::log as hlog;
use super::routing;

/// Префикс имени TUN-адаптера. По умолчанию sing-box стартует с
/// `nemefisto-<pid>`. Mihomo built-in TUN использует имя из YAML
/// (typically `Meta`, но мы не переопределяем — для mihomo детект
/// идёт по description либо IP).
const TUN_NAME_PREFIX: &str = "nemefisto-";
/// Адрес TUN-интерфейса от tun2proxy-эпохи (0.1.1 и ранее). Сейчас
/// sing-box создаёт TUN с другим IP по умолчанию, но half-routes на этот
/// IP могут остаться от старых сессий — чистим.
const TUN_GATEWAY: &str = "198.18.0.1";
const HALF_LOW_DST: &str = "0.0.0.0";
const HALF_HIGH_DST: &str = "128.0.0.0";
const HALF_MASK: &str = "128.0.0.0";

/// Опер-статус «адаптер поднят и работает». MIB_IF_ROW2.OperStatus
/// принимает значения IfOperStatusUp=1, Down=2, Testing=3, ... — нам
/// нужна только Up. Значение из MS-документации.
const IF_OPER_STATUS_UP: i32 = 1;

/// Найти индекс активного TUN-адаптера НАШЕГО движка.
///
/// Если `expect_tun=false` (proxy-режим) — single-shot, мгновенный None.
/// Если `expect_tun=true` (TUN-режим) — retry до 5с (адаптер появляется
/// ~500ms-2s после спавна sing-box/mihomo через helper).
pub async fn current_tun_interface_index(expect_tun: bool) -> Option<u32> {
    if !expect_tun {
        // Proxy-режим: TUN-адаптера быть не должно. Если от прошлой
        // TUN-сессии остался stale `nemefisto-*` адаптер (sing-box умер
        // не успев его почистить, OperStatus всё ещё Up), мы НЕ должны
        // добавлять allow-фильтр для него — kill-switch получится с
        // мёртвым LUID, FwpmFilterAdd0 валит транзакцию целиком.
        // Просто возвращаем None, никакого scan'а.
        hlog("[helper-tun] current_tun_interface_index: proxy-режим, TUN-поиск пропущен");
        return None;
    }
    hlog("[helper-tun] current_tun_interface_index: TUN-режим, ищем активный адаптер");
    let max_attempts = 50;
    for attempt in 0..max_attempts {
        let res = tokio::task::spawn_blocking(find_our_tun_interface_index_blocking).await;
        if let Ok(Some(idx)) = res {
            hlog(&format!(
                "[helper-tun] TUN-адаптер найден после {}мс retry, ifIndex={idx}",
                attempt * 100
            ));
            return Some(idx);
        }
        if attempt + 1 < max_attempts {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    hlog("[helper-tun] TUN-адаптер не появился за 5с retry — kill-switch без TUN allow!");
    None
}

/// Кандидат-адаптер: индекс + alias + description (для логов).
#[derive(Debug, Clone)]
struct TunCandidate {
    if_index: u32,
    alias: String,
    description: String,
    matched_by_alias: bool,
}

/// Многокритериальный синхронный поиск нашего TUN-адаптера.
///
/// **Доверенные критерии** (только эти возвращают Some):
/// 1. IP в `198.18.0.0/15` — RFC 2544 benchmark range, оба наших движка
///    (sing-box default 198.18.0.1/15, mihomo default 198.18.0.1/16)
///    сидят там. WireGuard / OpenVPN / других VPN тут НЕ бывает —
///    диапазон практически зарезервирован.
/// 2. Alias начинается с `nemefisto-` — наша явная подпись TUN-имени.
///
/// **Description-match** (sing-tun / wintun / WireGuard Tunnel / Mihomo)
/// — НЕ доверенный, потому что у юзера может быть параллельно работающий
/// WireGuard или другой VPN с тем же `Description`. Используется только
/// для логов («увидели но не взяли»).
///
/// Если ничего доверенного не найдено — возвращаем None, helper делает
/// retry. За 5с движок успеет создать адаптер и назначить IP.
fn find_our_tun_interface_index_blocking() -> Option<u32> {
    let scan = scan_interfaces();

    // 1. IP-таблица — самый надёжный признак.
    if let Some(ip_idx) = find_interface_with_ip_in_our_range() {
        if let Some(c) = scan.nemefisto_aliased.iter().find(|c| c.if_index == ip_idx) {
            hlog(&format!(
                "[helper-tun] выбран по IP 198.18.0.0/15 + alias `nemefisto-`: ifIndex={} alias={:?} desc={:?}",
                c.if_index, c.alias, c.description
            ));
        } else {
            // 12.E маскированное имя (`wlan99` / `Local Area Connection N`)
            // — alias не nemefisto-, но IP наш → точно наш TUN.
            hlog(&format!(
                "[helper-tun] выбран по IP 198.18.0.0/15 (без nemefisto-alias, маскировка 12.E?): ifIndex={ip_idx}"
            ));
        }
        return Some(ip_idx);
    }

    // 2. Без IP-match — пробуем alias prefix. Это случай когда движок
    //    создал адаптер, но IP ещё не назначен (race на старте sing-box).
    //    На следующих retry будет IP — но если адаптер уже nemefisto-
    //    то и так наш.
    if !scan.nemefisto_aliased.is_empty() {
        let c = &scan.nemefisto_aliased[0];
        if scan.nemefisto_aliased.len() > 1 {
            hlog(&format!(
                "[helper-tun] {} nemefisto-кандидатов без IP-match — берём первого: ifIndex={} alias={:?}",
                scan.nemefisto_aliased.len(),
                c.if_index,
                c.alias
            ));
        } else {
            hlog(&format!(
                "[helper-tun] выбран по alias `nemefisto-` (IP ещё не назначен): ifIndex={} alias={:?} desc={:?}",
                c.if_index, c.alias, c.description
            ));
        }
        return Some(c.if_index);
    }

    // 3. Только description-кандидаты (WireGuard, sing-tun-clones и т.п.) —
    //    НЕ доверяем. Логируем для диагностики и возвращаем None
    //    (helper сделает retry, движок успеет создать настоящий TUN).
    if !scan.description_only.is_empty() {
        hlog(&format!(
            "[helper-tun] {} description-only кандидатов (вероятно чужой VPN), пропускаем — ждём наш TUN",
            scan.description_only.len()
        ));
    }
    None
}

#[derive(Debug, Default)]
struct ScanResult {
    /// Адаптеры с alias начинающимся на `nemefisto-` — точно наши.
    nemefisto_aliased: Vec<TunCandidate>,
    /// Адаптеры с подозрительным description (WireGuard / sing-tun /
    /// wintun / Mihomo), но БЕЗ нашего alias-prefix. Не доверяем.
    description_only: Vec<TunCandidate>,
}

/// Перебор всех интерфейсов через `GetIfTable2`. Разделяет результат
/// на две группы: `nemefisto-`-aliased (доверяем) и description-only
/// (логируем, не выбираем).
fn scan_interfaces() -> ScanResult {
    let mut result = ScanResult::default();
    let mut table_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    let ret = unsafe { GetIfTable2(&mut table_ptr) };
    if ret != NO_ERROR || table_ptr.is_null() {
        hlog(&format!("[helper-tun] GetIfTable2 → код {ret}"));
        return result;
    }

    let table = unsafe { &*table_ptr };
    let entries = unsafe {
        std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize)
    };

    for entry in entries {
        if entry.OperStatus != IF_OPER_STATUS_UP {
            continue;
        }
        let alias = wide_z_to_string(&entry.Alias);
        let description = wide_z_to_string(&entry.Description);

        let alias_match = alias.starts_with(TUN_NAME_PREFIX);
        let desc_match = description_looks_like_our_tun(&description);

        if alias_match {
            result.nemefisto_aliased.push(TunCandidate {
                if_index: entry.InterfaceIndex,
                alias,
                description,
                matched_by_alias: true,
            });
        } else if desc_match {
            result.description_only.push(TunCandidate {
                if_index: entry.InterfaceIndex,
                alias,
                description,
                matched_by_alias: false,
            });
        }
    }

    hlog(&format!(
        "[helper-tun] scan: nemefisto-aliased={} description-only={} (всего {} интерфейсов)",
        result.nemefisto_aliased.len(),
        result.description_only.len(),
        table.NumEntries
    ));
    for c in &result.nemefisto_aliased {
        hlog(&format!(
            "  [TRUSTED nemefisto-] ifIndex={} alias={:?} desc={:?}",
            c.if_index, c.alias, c.description
        ));
    }
    for c in &result.description_only {
        hlog(&format!(
            "  [skip — desc-only, чужой VPN?] ifIndex={} alias={:?} desc={:?}",
            c.if_index, c.alias, c.description
        ));
    }

    unsafe { FreeMibTable(table_ptr as *mut _) };
    result
}

/// Проверка description адаптера на принадлежность к нашему TUN.
/// sing-box ставит description="sing-box", mihomo — "Mihomo",
/// generic Wintun-форки — "Wintun ...". Любой match — наш.
fn description_looks_like_our_tun(description: &str) -> bool {
    let lower = description.to_lowercase();
    lower.contains("sing-box")
        || lower.contains("mihomo")
        || lower.contains("wintun")
        || lower.contains("wireguard tunnel")
}

/// Найти индекс интерфейса с IPv4 адресом в `198.18.0.0/15` диапазоне.
/// Это RFC 2544 benchmark range — оба наших движка (sing-box default
/// 198.18.0.1/15, mihomo default 198.18.0.1/16) сидят тут.
///
/// `GetUnicastIpAddressTable(AF_INET)` — синхронный системный вызов
/// (~2мс), возвращает все unicast IP всех интерфейсов.
fn find_interface_with_ip_in_our_range() -> Option<u32> {
    let mut table_ptr: *mut MIB_UNICASTIPADDRESS_TABLE = std::ptr::null_mut();
    // AF_INET — только IPv4 (наш TUN range — v4). AF_UNSPEC вернёт
    // и v6, не нужно.
    let ret = unsafe { GetUnicastIpAddressTable(AF_INET, &mut table_ptr) };
    if ret != NO_ERROR || table_ptr.is_null() {
        hlog(&format!("[helper-tun] GetUnicastIpAddressTable → код {ret}"));
        return None;
    }

    let table = unsafe { &*table_ptr };
    let entries = unsafe {
        std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize)
    };

    let mut result: Option<u32> = None;
    for entry in entries {
        // Address — SOCKADDR_INET. Берём sin_family.
        let family = unsafe { entry.Address.si_family };
        if family != AF_INET {
            continue;
        }
        let v4: &SOCKADDR_IN = unsafe { mem::transmute(&entry.Address) };
        let raw = unsafe { v4.sin_addr.S_un.S_addr };
        let octets = raw.to_ne_bytes();
        // 198.18.0.0/15 — первый октет 198, второй 18 или 19.
        if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
            let ip = Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
            hlog(&format!(
                "[helper-tun] IP {} на ifIndex={} (наш TUN-диапазон)",
                ip, entry.InterfaceIndex
            ));
            result = Some(entry.InterfaceIndex);
            break;
        }
    }

    unsafe { FreeMibTable(table_ptr as *mut _) };
    let _ = AF_UNSPEC; // silence import warning if compiler complains
    result
}

/// `[u16; N]` с возможным null-terminator → `String`. Обрезает до
/// первого нулевого слова (если есть).
fn wide_z_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_looks_like_our_tun_matches() {
        let cases = [
            "sing-box",
            "Sing-Box",
            "Mihomo",
            "MIHOMO",
            "Wintun Userspace Tunnel",
            "WireGuard Tunnel via Wintun",
        ];
        for s in cases {
            assert!(
                description_looks_like_our_tun(s),
                "ожидаем match для: {s}"
            );
        }
    }

    #[test]
    fn description_looks_like_our_tun_rejects_unrelated() {
        let cases = [
            "Realtek PCIe GbE Family Controller",
            "Intel(R) Wi-Fi 6 AX201 160MHz",
            "Microsoft Wi-Fi Direct Virtual Adapter",
            "TAP-Windows Adapter V9",
            "Hyper-V Virtual Ethernet Adapter",
        ];
        for s in cases {
            assert!(
                !description_looks_like_our_tun(s),
                "не должны матчить: {s}"
            );
        }
    }

    #[test]
    fn wide_z_to_string_handles_null_terminator() {
        let mut buf: Vec<u16> = "nemefisto-1234".encode_utf16().collect();
        buf.push(0);
        // Дополняем мусором после null — должны его проигнорировать.
        buf.extend_from_slice(&[0xDEAD, 0xBEEF, 0]);
        assert_eq!(wide_z_to_string(&buf), "nemefisto-1234");
    }

    #[test]
    fn wide_z_to_string_handles_no_null() {
        let buf: Vec<u16> = "no-null".encode_utf16().collect();
        assert_eq!(wide_z_to_string(&buf), "no-null");
    }

    #[test]
    fn alias_prefix_const_is_lowercase_safe() {
        // Sanity: в коде сравниваем `alias.starts_with(TUN_NAME_PREFIX)`
        // case-sensitive — конфиги используют именно "nemefisto-" в
        // нижнем регистре, не "Nemefisto-". Если когда-нибудь поменяется —
        // тест поймает.
        assert_eq!(TUN_NAME_PREFIX, "nemefisto-");
    }
}
