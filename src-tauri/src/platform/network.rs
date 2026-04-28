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
