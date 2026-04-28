//! Управление маршрутизацией Windows напрямую через Win32 IP Helper API.
//!
//! Заменяет ранее использовавшиеся внешние команды (`Get-NetRoute`,
//! `route.exe`, `Set-DnsClientServerAddress`), каждая из которых тратила
//! 100–300 мс на запуск процесса. Теперь все операции — синхронные
//! системные вызовы (~1 мс каждый).
//!
//! Что используем:
//! - `GetIpForwardTable2` — чтение IPv4 routing-таблицы.
//! - `CreateIpForwardEntry2` / `DeleteIpForwardEntry2` — управление маршрутами.
//! - `ConvertInterfaceAliasToLuid` / `ConvertInterfaceLuidToIndex`
//!   `ConvertInterfaceLuidToAlias` — резолв имени/индекса/LUID.
//! - DNS пока через `netsh` — `SetInterfaceDnsSettings` доступен только с
//!   Win11 build 22000+; netsh работает на всех Windows и тратит ~150 мс.

use std::ffi::OsString;
use std::mem;
use std::net::Ipv4Addr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use tokio::process::Command as AsyncCommand;
use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, ERROR_OBJECT_ALREADY_EXISTS, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToAlias, ConvertInterfaceLuidToIndex,
    CreateIpForwardEntry2, CreateUnicastIpAddressEntry, DeleteIpForwardEntry2,
    DeleteUnicastIpAddressEntry, FreeMibTable, GetIfEntry2, GetIpForwardTable2,
    InitializeIpForwardEntry, InitializeUnicastIpAddressEntry, MIB_IF_ROW2, MIB_IPFORWARD_ROW2,
    MIB_IPFORWARD_TABLE2, MIB_UNICASTIPADDRESS_ROW,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Networking::WinSock::{
    AF_INET, IN_ADDR, IN_ADDR_0, SOCKADDR_IN, SOCKADDR_INET,
};

/// `NL_ROUTE_PROTOCOL_NETMGMT` — статически добавленный администратором маршрут.
/// В windows-sys не экспортирован; значение из ipdef.h Microsoft SDK.
/// Тип i32, потому что MIB_IPFORWARD_ROW2.Protocol тоже i32 (NL_ROUTE_PROTOCOL).
const MIB_IPPROTO_NETMGMT: i32 = 3;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DefaultRoute {
    pub gateway: String,
    pub if_index: u32,
    pub interface_name: String,
    pub luid: u64,
}

// ── Утилиты ────────────────────────────────────────────────────────────────

fn os_to_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = OsString::from(s).encode_wide().collect();
    v.push(0);
    v
}

fn ipv4_from_addr(addr: &SOCKADDR_INET) -> Option<Ipv4Addr> {
    unsafe {
        if addr.si_family != AF_INET {
            return None;
        }
        let v4: &SOCKADDR_IN = mem::transmute(addr);
        let raw = v4.sin_addr.S_un.S_addr; // network byte order (BE)
        let octets = raw.to_ne_bytes();
        Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
    }
}

fn make_sockaddr_inet_v4(addr: Ipv4Addr) -> SOCKADDR_INET {
    let octets = addr.octets();
    let raw = u32::from_ne_bytes(octets);
    let mut sa: SOCKADDR_INET = unsafe { mem::zeroed() };
    let v4: &mut SOCKADDR_IN = unsafe { mem::transmute(&mut sa) };
    v4.sin_family = AF_INET;
    v4.sin_port = 0;
    v4.sin_addr = IN_ADDR {
        S_un: IN_ADDR_0 { S_addr: raw },
    };
    sa
}

fn luid_from_index(if_index: u32) -> Result<u64> {
    let mut luid: NET_LUID_LH = unsafe { mem::zeroed() };
    let ret = unsafe {
        windows_sys::Win32::NetworkManagement::IpHelper::ConvertInterfaceIndexToLuid(if_index, &mut luid)
    };
    if ret != NO_ERROR {
        bail!("ConvertInterfaceIndexToLuid({if_index}) → код {ret}");
    }
    Ok(unsafe { luid.Value })
}

fn alias_from_luid(luid: u64) -> Result<String> {
    let mut net_luid: NET_LUID_LH = unsafe { mem::zeroed() };
    net_luid.Value = luid;
    let mut buf = vec![0u16; 256];
    let ret = unsafe { ConvertInterfaceLuidToAlias(&net_luid, buf.as_mut_ptr(), buf.len()) };
    if ret != NO_ERROR {
        bail!("ConvertInterfaceLuidToAlias → код {ret}");
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(OsString::from_wide(&buf[..len]).to_string_lossy().into_owned())
}

// ── Получить default-route ────────────────────────────────────────────────

pub async fn get_default_route() -> Result<DefaultRoute> {
    // GetIpForwardTable2 — синхронный вызов, занимает <5 мс.
    // Запускаем в spawn_blocking чтобы не блокировать tokio runtime.
    tokio::task::spawn_blocking(get_default_route_blocking)
        .await
        .context("spawn_blocking")?
}

fn get_default_route_blocking() -> Result<DefaultRoute> {
    let mut table_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    let ret = unsafe { GetIpForwardTable2(AF_INET, &mut table_ptr) };
    if ret != NO_ERROR {
        bail!("GetIpForwardTable2 → код {ret}");
    }
    if table_ptr.is_null() {
        bail!("GetIpForwardTable2 вернул NULL");
    }

    let table = unsafe { &*table_ptr };
    let entries =
        unsafe { std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize) };

    // Ищем default-route с минимальной метрикой и валидным NextHop.
    let mut best: Option<&MIB_IPFORWARD_ROW2> = None;
    for entry in entries {
        // 0.0.0.0/0
        let prefix_len = entry.DestinationPrefix.PrefixLength;
        let prefix_addr = ipv4_from_addr(&entry.DestinationPrefix.Prefix);
        if prefix_len != 0 || prefix_addr != Some(Ipv4Addr::new(0, 0, 0, 0)) {
            continue;
        }
        let next_hop = ipv4_from_addr(&entry.NextHop);
        if next_hop.is_none() || next_hop == Some(Ipv4Addr::new(0, 0, 0, 0)) {
            continue;
        }
        match best {
            None => best = Some(entry),
            Some(b) if entry.Metric < b.Metric => best = Some(entry),
            _ => {}
        }
    }

    let result = match best {
        Some(row) => {
            let gateway = ipv4_from_addr(&row.NextHop)
                .ok_or_else(|| anyhow!("NextHop не IPv4"))?
                .to_string();
            let if_index = row.InterfaceIndex;
            let luid = unsafe { row.InterfaceLuid.Value };
            let interface_name = alias_from_luid(luid)?;
            Ok(DefaultRoute {
                gateway,
                if_index,
                interface_name,
                luid,
            })
        }
        None => Err(anyhow!("default-route не найден")),
    };

    unsafe { FreeMibTable(table_ptr as *mut _) };
    result
}

// ── Резолв имени → IPv4 ───────────────────────────────────────────────────

pub async fn resolve_host_ipv4(host: &str) -> Result<String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(host.to_string());
    }
    let host_owned = host.to_string();
    let ip = tokio::task::spawn_blocking(move || -> Result<String> {
        use std::net::ToSocketAddrs;
        let addrs = (host_owned.as_str(), 443u16)
            .to_socket_addrs()
            .with_context(|| format!("резолв {host_owned} не удался"))?;
        for a in addrs {
            if let std::net::IpAddr::V4(v4) = a.ip() {
                return Ok(v4.to_string());
            }
        }
        Err(anyhow!("резолв {host_owned} не дал ни одного IPv4"))
    })
    .await??;
    Ok(ip)
}

// ── Дождаться появления интерфейса по alias ───────────────────────────────

/// Ждём не просто появления интерфейса, а его полной готовности —
/// `OperStatus == IfOperStatusUp` (1) и `MediaConnectState == Connected` (1).
/// Это решает race с tun2socks: между моментом когда WinTUN регистрирует
/// adapter и когда он готов принимать IP-настройки проходит ~700ms.
pub async fn wait_for_interface(name: &str, timeout: Duration) -> Result<u32> {
    const IF_OPER_STATUS_UP: i32 = 1;

    let deadline = Instant::now() + timeout;
    let wide_name = os_to_wide(name);
    loop {
        let mut luid: NET_LUID_LH = unsafe { mem::zeroed() };
        let ret_luid = unsafe { ConvertInterfaceAliasToLuid(wide_name.as_ptr(), &mut luid) };
        if ret_luid == NO_ERROR {
            let mut idx: u32 = 0;
            let ret_idx = unsafe { ConvertInterfaceLuidToIndex(&luid, &mut idx) };
            if ret_idx == NO_ERROR && idx != 0 {
                // Проверяем OperStatus и MediaConnectState через GetIfEntry2
                let mut row: MIB_IF_ROW2 = unsafe { mem::zeroed() };
                row.InterfaceLuid = luid;
                row.InterfaceIndex = idx;
                let ret_row = unsafe { GetIfEntry2(&mut row) };
                if ret_row == NO_ERROR && row.OperStatus == IF_OPER_STATUS_UP {
                    return Ok(idx);
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("интерфейс «{name}» не стал готов за {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ── Add / delete route ────────────────────────────────────────────────────

pub async fn add_route(
    destination: &str,
    mask: &str,
    gateway: &str,
    metric: u16,
    if_index: Option<u32>,
) -> Result<()> {
    let dst: Ipv4Addr = destination
        .parse()
        .with_context(|| format!("invalid destination IP: {destination}"))?;
    let mask: Ipv4Addr = mask
        .parse()
        .with_context(|| format!("invalid mask: {mask}"))?;
    let gw: Ipv4Addr = gateway
        .parse()
        .with_context(|| format!("invalid gateway: {gateway}"))?;
    let prefix_len = mask_to_prefix_len(mask);

    let if_idx = if_index.ok_or_else(|| anyhow!("if_index обязателен"))?;
    let luid = luid_from_index(if_idx)?;
    eprintln!(
        "[routing] add_route {dst}/{prefix_len} via {gw} ifIndex={if_idx} metric={metric}"
    );

    // Retry на ERROR_NOT_FOUND (1168) — типичный код когда NextHop ещё
    // недоступен через интерфейс (TUN только что создан, IP не назначен).
    // Backoff: 100, 200, 400, 800, 1600 ms = до ~3 сек суммарно.
    let metric_u32 = metric as u32;
    let mut delay = Duration::from_millis(100);
    let max_attempts = 5;

    for attempt in 1..=max_attempts {
        let result = tokio::task::spawn_blocking(move || -> Result<u32, u32> {
            unsafe {
                let mut row: MIB_IPFORWARD_ROW2 = mem::zeroed();
                InitializeIpForwardEntry(&mut row);
                row.InterfaceLuid.Value = luid;
                row.InterfaceIndex = if_idx;
                row.DestinationPrefix.Prefix = make_sockaddr_inet_v4(dst);
                row.DestinationPrefix.PrefixLength = prefix_len;
                row.NextHop = make_sockaddr_inet_v4(gw);
                row.Metric = metric_u32;
                row.Protocol = MIB_IPPROTO_NETMGMT;

                let ret = CreateIpForwardEntry2(&row);
                if ret == NO_ERROR || ret == ERROR_OBJECT_ALREADY_EXISTS {
                    Ok(ret)
                } else {
                    Err(ret)
                }
            }
        })
        .await
        .context("spawn_blocking")?;

        match result {
            Ok(_) => return Ok(()),
            Err(1168) if attempt < max_attempts => {
                // ERROR_NOT_FOUND: TUN-интерфейс ещё не готов принимать routes.
                eprintln!(
                    "[routing] add_route {dst}/{prefix_len}: попытка {attempt}/{max_attempts}, NextHop ещё не готов, ждём {}ms",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(code) => {
                bail!("CreateIpForwardEntry2 → код {code}");
            }
        }
    }
    bail!("CreateIpForwardEntry2: исчерпан retry-budget")
}

pub async fn delete_route(destination: &str, mask: &str) -> Result<()> {
    let dst: Ipv4Addr = match destination.parse() {
        Ok(d) => d,
        Err(_) => return Ok(()), // некорректный destination — нечего удалять
    };
    let mask: Ipv4Addr = match mask.parse() {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    let prefix_len = mask_to_prefix_len(mask);

    tokio::task::spawn_blocking(move || -> Result<()> {
        // Чтобы удалить — нужно найти существующую запись в таблице с такой же
        // destination/prefix и удалить её. Иначе DeleteIpForwardEntry2 требует
        // полную копию row (включая правильный InterfaceLuid).
        let mut table_ptr: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        let ret = unsafe { GetIpForwardTable2(AF_INET, &mut table_ptr) };
        if ret != NO_ERROR || table_ptr.is_null() {
            return Ok(());
        }
        let table = unsafe { &*table_ptr };
        let entries = unsafe {
            std::slice::from_raw_parts(table.Table.as_ptr(), table.NumEntries as usize)
        };

        for entry in entries {
            let entry_dst = ipv4_from_addr(&entry.DestinationPrefix.Prefix);
            if entry.DestinationPrefix.PrefixLength == prefix_len
                && entry_dst == Some(dst)
            {
                let ret_del = unsafe { DeleteIpForwardEntry2(entry) };
                if ret_del != NO_ERROR && ret_del != ERROR_NOT_FOUND {
                    eprintln!(
                        "[routing] DeleteIpForwardEntry2({}/{}) → код {}",
                        dst, prefix_len, ret_del
                    );
                }
            }
        }

        unsafe { FreeMibTable(table_ptr as *mut _) };
        Ok(())
    })
    .await??;
    Ok(())
}

fn mask_to_prefix_len(mask: Ipv4Addr) -> u8 {
    let bits = u32::from(mask);
    bits.count_ones() as u8
}

// ── Управление IP-адресами интерфейса ─────────────────────────────────────

/// Назначить IPv4-адрес на интерфейс. Нужно после поднятия TUN, потому что
/// WinTUN создаёт интерфейс без IP — это задача приложения.
///
/// Retry на ERROR_NOT_FOUND (1168): интерфейс может быть зарегистрирован
/// в системе через 1-2ms после старта tun2socks, но физически ещё «не до
/// конца» создан — driver-callback не отработал.
pub async fn assign_ip(if_index: u32, addr: Ipv4Addr, prefix_len: u8) -> Result<()> {
    eprintln!("[routing] assign_ip {addr}/{prefix_len} ifIndex={if_index}");

    let mut delay = Duration::from_millis(100);
    let max_attempts = 6; // 100+200+400+800+1600 = ~3.1с

    for attempt in 1..=max_attempts {
        // На каждой попытке заново резолвим LUID — интерфейс мог только что появиться.
        let luid = match luid_from_index(if_index) {
            Ok(l) => l,
            Err(_) if attempt < max_attempts => {
                eprintln!(
                    "[routing] assign_ip: попытка {attempt}/{max_attempts}, LUID не резолвится, ждём {}ms",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                delay *= 2;
                continue;
            }
            Err(e) => return Err(e),
        };

        let result = tokio::task::spawn_blocking(move || -> Result<u32, u32> {
            unsafe {
                let mut row: MIB_UNICASTIPADDRESS_ROW = mem::zeroed();
                InitializeUnicastIpAddressEntry(&mut row);
                row.InterfaceLuid.Value = luid;
                row.InterfaceIndex = if_index;
                row.Address = make_sockaddr_inet_v4(addr);
                row.OnLinkPrefixLength = prefix_len;
                row.DadState = 4; // IpDadStatePreferred — пропускаем DAD для скорости
                row.ValidLifetime = 0xFFFFFFFF;
                row.PreferredLifetime = 0xFFFFFFFF;

                let ret = CreateUnicastIpAddressEntry(&row);
                if ret == NO_ERROR || ret == ERROR_OBJECT_ALREADY_EXISTS {
                    Ok(ret)
                } else {
                    Err(ret)
                }
            }
        })
        .await
        .context("spawn_blocking")?;

        match result {
            Ok(_) => return Ok(()),
            Err(1168) if attempt < max_attempts => {
                eprintln!(
                    "[routing] assign_ip: попытка {attempt}/{max_attempts}, интерфейс ещё не готов, ждём {}ms",
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(code) => bail!("CreateUnicastIpAddressEntry → код {code}"),
        }
    }
    bail!("assign_ip: исчерпан retry-budget")
}

/// Снять IP с интерфейса. Идемпотентно — отсутствие адреса не ошибка.
pub async fn unassign_ip(if_index: u32, addr: Ipv4Addr, prefix_len: u8) -> Result<()> {
    let luid = match luid_from_index(if_index) {
        Ok(l) => l,
        // Интерфейс уже исчез (tun2socks убит) — тоже считаем успехом
        Err(_) => return Ok(()),
    };

    tokio::task::spawn_blocking(move || -> Result<()> {
        unsafe {
            let mut row: MIB_UNICASTIPADDRESS_ROW = mem::zeroed();
            InitializeUnicastIpAddressEntry(&mut row);
            row.InterfaceLuid.Value = luid;
            row.InterfaceIndex = if_index;
            row.Address = make_sockaddr_inet_v4(addr);
            row.OnLinkPrefixLength = prefix_len;

            let ret = DeleteUnicastIpAddressEntry(&row);
            if ret == NO_ERROR || ret == ERROR_NOT_FOUND {
                Ok(())
            } else {
                Err(anyhow!("DeleteUnicastIpAddressEntry → код {ret}"))
            }
        }
    })
    .await??;
    Ok(())
}

// ── DNS ────────────────────────────────────────────────────────────────────

/// Установить DNS на интерфейс через netsh. Это запуск процесса (~150 мс),
/// но нативный `SetInterfaceDnsSettings` доступен только с Windows 11
/// build 22000, что не поддерживает Win10. Пока оставляем netsh.
pub async fn set_dns(if_index: u32, dns: &str) -> Result<()> {
    let alias = alias_from_luid(luid_from_index(if_index)?)?;
    let _ = AsyncCommand::new("netsh")
        .args([
            "interface",
            "ipv4",
            "set",
            "dnsservers",
            &alias,
            "static",
            dns,
            "primary",
            "validate=no",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .context("netsh не запустился")?;
    Ok(())
}

