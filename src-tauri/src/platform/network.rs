//! Определение physic-интерфейса через который идёт default-route.
//!
//! Используется для `streamSettings.sockopt.interface` Xray direct-outbound
//! в TUN-режиме (см. `xray_config::patch_xray_json`).

/// Возвращает имя (alias) физического интерфейса с минимальной метрикой
/// default-route — например `"Ethernet"` или `"Wi-Fi"`.
///
/// Используется для `streamSettings.sockopt.interface` в direct-outbound
/// Xray в TUN-режиме: Xray на Windows реализует эту опцию через
/// `IP_UNICAST_IF`, который заставляет ОС маршрутизировать **этот конкретный
/// сокет** через указанный интерфейс минуя routing-таблицу. Так мы обходим
/// наш half-default route через TUN — direct-трафик идёт через physic, а не
/// зацикливается через TUN → tun2socks → Xray → direct → loop.
///
/// `sendThrough` (bind-to-IP) на Windows не помогает из-за weak-host model:
/// ОС выбирает интерфейс по destination, source-IP игнорируется.
#[cfg(windows)]
pub fn get_default_route_interface_name() -> Option<String> {
    use std::mem;
    use windows_sys::Win32::Foundation::NO_ERROR;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToAlias, FreeMibTable,
        GetIpForwardTable2, MIB_IPFORWARD_TABLE2,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN, SOCKADDR_INET};

    unsafe {
        // Шаг 1: найти ifIndex с минимальной метрикой default-route
        let mut fwd_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        if GetIpForwardTable2(AF_INET, &mut fwd_ptr) != NO_ERROR || fwd_ptr.is_null() {
            return None;
        }
        let fwd = &*fwd_ptr;
        let fwd_entries = std::slice::from_raw_parts(fwd.Table.as_ptr(), fwd.NumEntries as usize);

        let mut best_if_idx: Option<u32> = None;
        let mut best_metric: u32 = u32::MAX;
        for entry in fwd_entries {
            if entry.DestinationPrefix.PrefixLength != 0 {
                continue;
            }
            let nh: &SOCKADDR_INET = &entry.NextHop;
            if nh.si_family != AF_INET {
                continue;
            }
            let nh_v4: &SOCKADDR_IN = mem::transmute(nh);
            if nh_v4.sin_addr.S_un.S_addr == 0 {
                continue;
            }
            if entry.Metric < best_metric {
                best_metric = entry.Metric;
                best_if_idx = Some(entry.InterfaceIndex);
            }
        }
        FreeMibTable(fwd_ptr as *mut _);
        let if_idx = best_if_idx?;

        // Шаг 2: ifIndex → LUID → Alias
        let mut luid: NET_LUID_LH = mem::zeroed();
        if ConvertInterfaceIndexToLuid(if_idx, &mut luid) != 0 {
            return None;
        }
        let mut alias_buf = [0u16; 256];
        if ConvertInterfaceLuidToAlias(&luid, alias_buf.as_mut_ptr(), alias_buf.len())
            != NO_ERROR
        {
            return None;
        }
        let len = alias_buf.iter().position(|&c| c == 0).unwrap_or(alias_buf.len());
        let alias = String::from_utf16_lossy(&alias_buf[..len]);
        if alias.is_empty() {
            None
        } else {
            Some(alias)
        }
    }
}

#[cfg(not(windows))]
pub fn get_default_route_interface_name() -> Option<String> {
    None
}

/// 9.C — Детект сторонних VPN по routing-таблице.
///
/// Алгоритм:
/// 1. Находим physic-default ifIndex (наименьшая metric, NextHop ≠ 0,
///    PrefixLength = 0) — это «штатный» Wi-Fi/Ethernet шлюз.
/// 2. Перебираем все default- и half-default-маршруты (PrefixLength
///    ∈ {0, 1}).
/// 3. Конфликтом считаем маршрут, у которого:
///    - NextHop ≠ 198.18.0.1 (не наш TUN-gateway);
///    - NextHop ≠ 0.0.0.0 (не on-link);
///    - ifIndex ≠ physic-default ifIndex.
/// 4. Резолвим alias интерфейса и возвращаем уникальные имена.
///
/// Возвращает пустой вектор если конфликтов нет / на не-Windows.
#[cfg(windows)]
pub fn detect_routing_conflicts() -> Vec<String> {
    use std::collections::HashSet;
    use std::mem;
    use windows_sys::Win32::Foundation::NO_ERROR;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToAlias, FreeMibTable,
        GetIpForwardTable2, MIB_IPFORWARD_TABLE2,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN, SOCKADDR_INET};

    // 198.18.0.1 в network-byte-order (так лежит в S_addr WinSock).
    // 198 = 0xC6, 18 = 0x12 → little-endian S_un.S_addr хранит как 0x010012C6.
    const NEMEFISTO_TUN_GATEWAY: u32 = 0x0100_12C6;

    unsafe {
        let mut fwd_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        if GetIpForwardTable2(AF_INET, &mut fwd_ptr) != NO_ERROR || fwd_ptr.is_null() {
            return Vec::new();
        }
        let fwd = &*fwd_ptr;
        let entries = std::slice::from_raw_parts(fwd.Table.as_ptr(), fwd.NumEntries as usize);

        // Шаг 1: physic-default ifIndex (минимальная метрика среди /0 с не-zero NextHop).
        let mut physic_if_idx: Option<u32> = None;
        let mut physic_metric = u32::MAX;
        for e in entries {
            if e.DestinationPrefix.PrefixLength != 0 {
                continue;
            }
            let nh: &SOCKADDR_INET = &e.NextHop;
            if nh.si_family != AF_INET {
                continue;
            }
            let nh_v4: &SOCKADDR_IN = mem::transmute(nh);
            let nh_addr = nh_v4.sin_addr.S_un.S_addr;
            if nh_addr == 0 || nh_addr == NEMEFISTO_TUN_GATEWAY {
                continue;
            }
            if e.Metric < physic_metric {
                physic_metric = e.Metric;
                physic_if_idx = Some(e.InterfaceIndex);
            }
        }

        // Шаг 2: ищем подозрительные default + half-default.
        let mut conflict_ifs: HashSet<u32> = HashSet::new();
        for e in entries {
            // half-default = /1 c destination 0.0.0.0 или 128.0.0.0 (типовой VPN-приём).
            let prefix_len = e.DestinationPrefix.PrefixLength;
            if prefix_len > 1 {
                continue;
            }
            let nh: &SOCKADDR_INET = &e.NextHop;
            if nh.si_family != AF_INET {
                continue;
            }
            let nh_v4: &SOCKADDR_IN = mem::transmute(nh);
            let nh_addr = nh_v4.sin_addr.S_un.S_addr;
            if nh_addr == 0 || nh_addr == NEMEFISTO_TUN_GATEWAY {
                continue;
            }
            if Some(e.InterfaceIndex) == physic_if_idx {
                continue;
            }
            conflict_ifs.insert(e.InterfaceIndex);
        }
        FreeMibTable(fwd_ptr as *mut _);

        // Шаг 3: резолвим aliases + фильтруем известные P2P / mesh-VPN
        // (Radmin, Hamachi, ZeroTier, Tailscale, Nebula, AnyConnect и
        // подобные). Они хотя и ставят default/half-default route'ы, но
        // не маршрутизируют общий трафик через интернет — это VPN-сети
        // между конкретными пирами / корпоративные mesh-узлы. С нашим
        // трафиком они не конкурируют, ругаться на них не надо.
        //
        // Сравнение case-insensitive по подстроке alias'а интерфейса.
        const FRIENDLY_TOKENS: &[&str] = &[
            "radmin",
            "hamachi",
            "zerotier",
            "tailscale",
            "nebula",
            "anyconnect",
            "softether",
            "logmein",
            "openvpn tap", // OpenVPN TAP-Windows adapter — обычно для корпоративных сетей
            "tap-windows",
        ];

        let mut aliases: Vec<String> = Vec::new();
        for if_idx in conflict_ifs {
            let mut luid: NET_LUID_LH = mem::zeroed();
            if ConvertInterfaceIndexToLuid(if_idx, &mut luid) != 0 {
                continue;
            }
            let mut alias_buf = [0u16; 256];
            if ConvertInterfaceLuidToAlias(&luid, alias_buf.as_mut_ptr(), alias_buf.len())
                != NO_ERROR
            {
                continue;
            }
            let len = alias_buf
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(alias_buf.len());
            let alias = String::from_utf16_lossy(&alias_buf[..len]);
            if alias.is_empty() {
                continue;
            }
            let lower = alias.to_lowercase();
            if FRIENDLY_TOKENS.iter().any(|tok| lower.contains(tok)) {
                continue;
            }
            aliases.push(alias);
        }
        aliases.sort();
        aliases.dedup();
        aliases
    }
}

#[cfg(not(windows))]
pub fn detect_routing_conflicts() -> Vec<String> {
    Vec::new()
}

/// 14.E — есть ли в системе orphan TUN-адаптер с префиксом `nemefisto-`.
///
/// Используется при показе recovery dialog'а: если адаптер от прошлой
/// упавшей сессии не убран — пользователь видит галку «orphan TUN-адаптер»
/// и может починить через `recover_network`.
///
/// Native `GetIfTable2` + проверка `MIB_IF_ROW2.Alias` (поле уже содержит
/// строку, не надо вызывать ConvertInterfaceLuidToAlias). Быстро, <10ms
/// даже на машинах с десятком интерфейсов.
#[cfg(windows)]
pub fn has_orphan_tun_adapters() -> bool {
    use windows_sys::Win32::Foundation::NO_ERROR;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };

    unsafe {
        let mut tbl_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut tbl_ptr) != NO_ERROR || tbl_ptr.is_null() {
            return false;
        }
        let tbl = &*tbl_ptr;
        let rows = std::slice::from_raw_parts(tbl.Table.as_ptr(), tbl.NumEntries as usize);
        let mut found = false;
        for row in rows {
            // Alias — это [u16; 257], читаем до первого нуля.
            let alias_len = row
                .Alias
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(row.Alias.len());
            let alias = String::from_utf16_lossy(&row.Alias[..alias_len]);
            if alias.to_lowercase().starts_with("nemefisto-") {
                found = true;
                break;
            }
        }
        FreeMibTable(tbl_ptr as *mut _);
        found
    }
}

#[cfg(not(windows))]
pub fn has_orphan_tun_adapters() -> bool {
    false
}

// ─── Routing table viewer (live diagnostic, read-only) ─────────────────────

/// Запись routing-таблицы для UI-вьюера. Сериализуется наружу.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouteEntry {
    /// `"v4"` | `"v6"`.
    pub family: &'static str,
    /// Назначение в формате CIDR: `"0.0.0.0/0"`, `"192.168.1.0/24"`, `"::1/128"`.
    pub destination: String,
    /// Next-hop IP или `"on-link"` если шлюза нет.
    pub next_hop: String,
    /// Friendly-имя интерфейса (`"Wi-Fi"`, `"nemefisto-1234"`, и т.д.).
    /// Если резолвер упал — fallback на `"if{index}"`.
    pub interface: String,
    /// `InterfaceIndex` — для группировки/фильтрации в UI.
    pub interface_index: u32,
    /// Метрика (приоритет: меньше = выше).
    pub metric: u32,
}

/// Чтение текущей routing-таблицы (IPv4 + IPv6) из ядра Windows для
/// UI-диагностики. Read-only, не модифицирует таблицу. ~5-10ms на
/// типичной машине с 10-30 маршрутами.
///
/// Используется командой `get_routing_table` для Settings → диагностика.
/// Помогает пользователю самому увидеть «куда уходит мой трафик» когда
/// что-то не работает (есть ли default через TUN, не остался ли orphan
/// маршрут от другого VPN, и т.п.).
#[cfg(windows)]
pub fn list_routing_table() -> Vec<RouteEntry> {
    use std::mem;
    use windows_sys::Win32::Foundation::NO_ERROR;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceIndexToLuid, ConvertInterfaceLuidToAlias, FreeMibTable,
        GetIpForwardTable2, MIB_IPFORWARD_TABLE2,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, AF_INET6, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_INET,
    };

    let mut out: Vec<RouteEntry> = Vec::new();

    // Кеш ifIndex → alias чтобы не дёргать ConvertInterfaceLuid* на каждый
    // route (одна и та же сетевая карта обычно засветится в 5+ маршрутах).
    let mut alias_cache: std::collections::HashMap<u32, String> =
        std::collections::HashMap::new();

    let resolve_alias = |if_idx: u32,
                         cache: &mut std::collections::HashMap<u32, String>|
     -> String {
        if let Some(s) = cache.get(&if_idx) {
            return s.clone();
        }
        unsafe {
            let mut luid: NET_LUID_LH = mem::zeroed();
            if ConvertInterfaceIndexToLuid(if_idx, &mut luid) != 0 {
                let s = format!("if{if_idx}");
                cache.insert(if_idx, s.clone());
                return s;
            }
            let mut buf = [0u16; 256];
            if ConvertInterfaceLuidToAlias(&luid, buf.as_mut_ptr(), buf.len()) != NO_ERROR {
                let s = format!("if{if_idx}");
                cache.insert(if_idx, s.clone());
                return s;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..len]);
            let s = if s.is_empty() { format!("if{if_idx}") } else { s };
            cache.insert(if_idx, s.clone());
            s
        }
    };

    // ── IPv4 ─────────────────────────────────────────────────────────────
    unsafe {
        let mut fwd_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        if GetIpForwardTable2(AF_INET, &mut fwd_ptr) == NO_ERROR && !fwd_ptr.is_null() {
            let fwd = &*fwd_ptr;
            let entries =
                std::slice::from_raw_parts(fwd.Table.as_ptr(), fwd.NumEntries as usize);
            for e in entries {
                let prefix_len = e.DestinationPrefix.PrefixLength;
                let dest = &e.DestinationPrefix.Prefix;
                if dest.si_family != AF_INET {
                    continue;
                }
                let dest_v4: &SOCKADDR_IN = mem::transmute(dest);
                let dest_addr = u32::from_be(dest_v4.sin_addr.S_un.S_addr);
                let dest_str = format!(
                    "{}.{}.{}.{}/{}",
                    (dest_addr >> 24) & 0xFF,
                    (dest_addr >> 16) & 0xFF,
                    (dest_addr >> 8) & 0xFF,
                    dest_addr & 0xFF,
                    prefix_len
                );

                let nh: &SOCKADDR_INET = &e.NextHop;
                let nh_str = if nh.si_family == AF_INET {
                    let nh_v4: &SOCKADDR_IN = mem::transmute(nh);
                    let nh_addr = u32::from_be(nh_v4.sin_addr.S_un.S_addr);
                    if nh_addr == 0 {
                        "on-link".to_string()
                    } else {
                        format!(
                            "{}.{}.{}.{}",
                            (nh_addr >> 24) & 0xFF,
                            (nh_addr >> 16) & 0xFF,
                            (nh_addr >> 8) & 0xFF,
                            nh_addr & 0xFF
                        )
                    }
                } else {
                    "on-link".to_string()
                };

                out.push(RouteEntry {
                    family: "v4",
                    destination: dest_str,
                    next_hop: nh_str,
                    interface: resolve_alias(e.InterfaceIndex, &mut alias_cache),
                    interface_index: e.InterfaceIndex,
                    metric: e.Metric,
                });
            }
            FreeMibTable(fwd_ptr as *mut _);
        }
    }

    // ── IPv6 ─────────────────────────────────────────────────────────────
    unsafe {
        let mut fwd_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        if GetIpForwardTable2(AF_INET6, &mut fwd_ptr) == NO_ERROR && !fwd_ptr.is_null() {
            let fwd = &*fwd_ptr;
            let entries =
                std::slice::from_raw_parts(fwd.Table.as_ptr(), fwd.NumEntries as usize);
            for e in entries {
                let prefix_len = e.DestinationPrefix.PrefixLength;
                let dest = &e.DestinationPrefix.Prefix;
                if dest.si_family != AF_INET6 {
                    continue;
                }
                let dest_v6: &SOCKADDR_IN6 = mem::transmute(dest);
                let dest_octets = dest_v6.sin6_addr.u.Byte;
                let dest_addr = std::net::Ipv6Addr::from(dest_octets);
                let dest_str = format!("{dest_addr}/{prefix_len}");

                let nh: &SOCKADDR_INET = &e.NextHop;
                let nh_str = if nh.si_family == AF_INET6 {
                    let nh_v6: &SOCKADDR_IN6 = mem::transmute(nh);
                    let nh_addr = std::net::Ipv6Addr::from(nh_v6.sin6_addr.u.Byte);
                    if nh_addr.is_unspecified() {
                        "on-link".to_string()
                    } else {
                        nh_addr.to_string()
                    }
                } else {
                    "on-link".to_string()
                };

                out.push(RouteEntry {
                    family: "v6",
                    destination: dest_str,
                    next_hop: nh_str,
                    interface: resolve_alias(e.InterfaceIndex, &mut alias_cache),
                    interface_index: e.InterfaceIndex,
                    metric: e.Metric,
                });
            }
            FreeMibTable(fwd_ptr as *mut _);
        }
    }

    // Сортируем по metric ASC, затем по destination — естественный порядок
    // «первый сработает» сверху.
    out.sort_by(|a, b| {
        a.metric
            .cmp(&b.metric)
            .then_with(|| a.destination.cmp(&b.destination))
    });
    out
}

#[cfg(not(windows))]
pub fn list_routing_table() -> Vec<RouteEntry> {
    Vec::new()
}
