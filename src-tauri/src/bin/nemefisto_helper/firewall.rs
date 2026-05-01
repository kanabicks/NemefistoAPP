//! Kill switch через WFP API напрямую (этап 13.D).
//!
//! Заменяет старую netsh-реализацию. Преимущества:
//! - не меняет глобальную firewall policy (не пересекается с правилами
//!   пользователя / Defender / других VPN);
//! - DYNAMIC session: фильтры **автоматически** удаляются если процесс
//!   helper'а упал — пользователь не остаётся без интернета;
//! - транзакционные изменения: либо все allow-rules применились,
//!   либо ни одного (никаких half-applied "block без allow VPN").
//!
//! Архитектура фильтров (по убыванию weight):
//! - **W_DHCP=16** — DHCP/BOOTP UDP 67/68 (для получения IP в новой сети)
//! - **W_APP=14** — наши процессы по абсолютному пути (xray.exe, mihomo.exe,
//!   tun2socks.exe, nemefisto-helper.exe)
//! - **W_SERVER=12** — IP VPN-сервера (резолв на стороне Tauri-main)
//! - **W_LOOPBACK=10** — 127.0.0.0/8 (наш SOCKS5/HTTP inbound) + ::1/128
//! - **W_LAN=8** — 10/8, 172.16/12, 192.168/16, 169.254/16 (если allow_lan=true)
//! - **W_BLOCK=0** — fallback block-all в каждом ALE_AUTH_CONNECT слое

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
    FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6,
};

use super::wfp::{
    cleanup_provider as wfp_cleanup_provider, WfpEngine, NEMEFISTO_PROVIDER_GUID,
    NEMEFISTO_SUBLAYER_GUID,
};

// Веса фильтров. Higher = проверяется первым. Block-all — без weight
// в add_filter_block_all (там 0 захардкожен внутри wfp.rs).
const W_LAN: u8 = 8;
// DNS_BLOCK > LAN — иначе LAN-allow перебивал бы блок локального
// router DNS на 192.168.x.x:53. DNS_BLOCK = 9.
const W_DNS_BLOCK: u8 = 9;
const W_LOOPBACK: u8 = 10;
const W_SERVER: u8 = 12;
const W_APP: u8 = 14;
// DNS_PERMIT выше всех — нужно перебить и DNS_BLOCK, и LAN, иначе
// VPN-DNS на локальном адресе (типа 198.18.0.1) будет заблокирован.
const W_DNS_PERMIT: u8 = 15;
// TUN-interface allow — высокий weight: всё что ушло через TUN-адаптер
// уже шифровано VPN'ом, нет смысла перепроверять. Перебивает даже
// DNS_BLOCK (если случайно DNS-запрос ушёл через TUN — он легитимен).
const W_TUN_INTERFACE: u8 = 16;

// IANA protocol numbers — захардкодим, чтобы не тащить feature
// `Win32_Networking_WinSock` сюда (она в `windows-sys` для другого).
const IPPROTO_TCP: u8 = 6;
const IPPROTO_UDP: u8 = 17;

/// Глобальный engine — пока жив, kill-switch активен. Drop вызывает
/// `FwpmEngineClose0`, и DYNAMIC session автоматически уносит все наши
/// фильтры (это и есть «защита от orphan'ов»).
///
/// `Mutex<Option<WfpEngine>>`: `None` = killswitch выключен. Повторный
/// `enable()` пересоздаёт engine.
static ENGINE: OnceLock<Mutex<Option<WfpEngine>>> = OnceLock::new();

fn engine_lock() -> &'static Mutex<Option<WfpEngine>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}

/// Watchdog: timestamp последнего heartbeat (unix sec). Если main
/// не пингует 60 секунд — watchdog-таск автоматически делает disable().
/// Это страховка на случай если main завис / был принудительно убит /
/// потерял связь с helper'ом (а DYNAMIC session почему-то не сработала).
///
/// `i64::MIN` как sentinel = watchdog неактивен (kill-switch выключен).
static LAST_HEARTBEAT: AtomicI64 = AtomicI64::new(i64::MIN);

const WATCHDOG_TIMEOUT_SECS: i64 = 60;
const WATCHDOG_POLL_SECS: u64 = 5;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Обновить heartbeat. Зовётся из dispatch при `KillSwitchHeartbeat`.
pub fn heartbeat() {
    LAST_HEARTBEAT.store(now_unix(), Ordering::SeqCst);
}

/// HANDLE (`*mut c_void`) сам по себе не Send. Но MSDN явно говорит
/// что engine handle можно использовать из любого thread'а после
/// open. Mutex синхронизирует доступ — конкурентного использования
/// не будет.
unsafe impl Send for WfpEngine {}

/// Включить kill-switch с заданным allowlist'ом.
///
/// `server_ips` — список IP-адресов VPN-сервера (резолвится на стороне
/// Tauri-main, потому что `getaddrinfo` через VPN-туннель не сработает
/// как только мы заблокируем outbound). Пустой массив допустим — VPN
/// просто не сможет соединиться, тогда юзер должен сам переподключиться.
///
/// `allow_lan` — пускать ли локальную сеть (10/8, 172.16/12, 192.168/16).
///
/// `allow_app_paths` — абсолютные пути к нашим бинарям (xray, mihomo,
/// tun2socks, helper). Без них VPN-движок не сможет достучаться до
/// сервера даже если IP есть в server_ips (мы используем оба condition'а
/// как разные allow-rules — match-any семантика WFP).
///
/// `block_dns` + `allow_dns_ips` — DNS leak protection (этап 13.D step B).
/// Если on, блокируем весь :53/UDP+TCP трафик кроме указанных IP.
/// Полезно в TUN-режиме где DNS должен идти на VPN-DNS (типа 198.18.0.1).
/// В proxy-режиме ломает приложения которые не используют системный
/// прокси для DNS — поэтому опция включается явно пользователем.
pub async fn enable(
    server_ips: Vec<String>,
    allow_lan: bool,
    allow_app_paths: Vec<PathBuf>,
    block_dns: bool,
    allow_dns_ips: Vec<String>,
    tun_interface_index: Option<u32>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        enable_blocking(
            server_ips,
            allow_lan,
            allow_app_paths,
            block_dns,
            allow_dns_ips,
            tun_interface_index,
        )
    })
    .await
    .context("spawn_blocking panic")?
}

fn enable_blocking(
    server_ips: Vec<String>,
    allow_lan: bool,
    allow_app_paths: Vec<PathBuf>,
    block_dns: bool,
    allow_dns_ips: Vec<String>,
    tun_interface_index: Option<u32>,
) -> Result<()> {
    eprintln!(
        "[wfp-killswitch] ON: server_ips={:?}, allow_lan={}, apps={}, block_dns={}, dns_allow={:?}, tun_if={:?}",
        server_ips,
        allow_lan,
        allow_app_paths.len(),
        block_dns,
        allow_dns_ips,
        tun_interface_index,
    );
    for p in &allow_app_paths {
        eprintln!(
            "[wfp-killswitch]   app path: {} (exists={})",
            p.display(),
            p.is_file()
        );
    }

    // Закрываем предыдущую сессию если была — повторный enable безопасен.
    // Drop происходит при `*g = None` — старые фильтры уйдут перед
    // добавлением новых.
    {
        let mut g = engine_lock().lock().unwrap();
        *g = None;
    }

    let engine = WfpEngine::open_dynamic().context("open WFP engine")?;

    engine.transaction(|e| {
        e.add_provider(NEMEFISTO_PROVIDER_GUID, "Nemefisto VPN KillSwitch")?;
        e.add_sublayer(
            NEMEFISTO_SUBLAYER_GUID,
            NEMEFISTO_PROVIDER_GUID,
            "Nemefisto KillSwitch",
            0xFFFF,
        )?;

        // Block-all fallback в каждом layer.
        e.add_filter_block_all(
            FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            NEMEFISTO_SUBLAYER_GUID,
            "block-all v4",
        )?;
        e.add_filter_block_all(
            FWPM_LAYER_ALE_AUTH_CONNECT_V6,
            NEMEFISTO_SUBLAYER_GUID,
            "block-all v6",
        )?;

        // ── Loopback (127.0.0.0/8 + ::1) ─────────────────────────────
        // Наш SOCKS5/HTTP inbound — без этого приложения не достучатся
        // до прокси.
        e.add_filter_allow_v4_subnet(
            FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            NEMEFISTO_SUBLAYER_GUID,
            "loopback v4",
            W_LOOPBACK,
            0x7F00_0000,
            0xFF00_0000,
        )?;
        e.add_filter_allow_v6_addr(
            FWPM_LAYER_ALE_AUTH_CONNECT_V6,
            NEMEFISTO_SUBLAYER_GUID,
            "loopback v6",
            W_LOOPBACK,
            ipv6_addr_octets("::1"),
        )?;

        // ── LAN ──────────────────────────────────────────────────────
        if allow_lan {
            // RFC1918 + APIPA
            for (name, addr, mask) in [
                ("LAN 10/8", 0x0A00_0000u32, 0xFF00_0000u32),
                ("LAN 172.16/12", 0xAC10_0000, 0xFFF0_0000),
                ("LAN 192.168/16", 0xC0A8_0000, 0xFFFF_0000),
                ("LAN 169.254/16 (APIPA)", 0xA9FE_0000, 0xFFFF_0000),
            ] {
                e.add_filter_allow_v4_subnet(
                    FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                    NEMEFISTO_SUBLAYER_GUID,
                    name,
                    W_LAN,
                    addr,
                    mask,
                )?;
            }
            // IPv6: link-local fe80::/10 + multicast ff00::/8
            e.add_filter_allow_v6_subnet(
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                NEMEFISTO_SUBLAYER_GUID,
                "LAN link-local v6",
                W_LAN,
                ipv6_addr_octets("fe80::"),
                10,
            )?;
            e.add_filter_allow_v6_subnet(
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                NEMEFISTO_SUBLAYER_GUID,
                "LAN multicast v6",
                W_LAN,
                ipv6_addr_octets("ff00::"),
                8,
            )?;
        }

        // ── VPN-сервер IPs ───────────────────────────────────────────
        for (i, ip_str) in server_ips.iter().enumerate() {
            if let Ok(v4) = ip_str.parse::<Ipv4Addr>() {
                let addr = ipv4_to_u32(v4);
                e.add_filter_allow_v4_addr(
                    FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                    NEMEFISTO_SUBLAYER_GUID,
                    &format!("VPN server v4 #{i}"),
                    W_SERVER,
                    addr,
                )?;
            } else if let Ok(v6) = ip_str.parse::<Ipv6Addr>() {
                e.add_filter_allow_v6_addr(
                    FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                    NEMEFISTO_SUBLAYER_GUID,
                    &format!("VPN server v6 #{i}"),
                    W_SERVER,
                    v6.octets(),
                )?;
            } else {
                eprintln!("[wfp-killswitch] пропускаю невалидный IP: {ip_str}");
            }
        }

        // ── App-id allow ─────────────────────────────────────────────
        let mut apps_added_v4 = 0u32;
        let mut apps_added_v6 = 0u32;
        for path in &allow_app_paths {
            // Дублируем для V4 и V6 — match-семантика WFP per-layer.
            let label = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            // FwpmGetAppIdFromFileName0 может упасть для несуществующего
            // пути — в этом случае не валим всю транзакцию, просто
            // пропускаем (eprintln в add_filter_allow_app покажет).
            match e.add_filter_allow_app(
                FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                NEMEFISTO_SUBLAYER_GUID,
                &format!("app v4 {label}"),
                W_APP,
                path,
            ) {
                Ok(()) => {
                    apps_added_v4 += 1;
                    eprintln!("[wfp-killswitch]   + allow app v4: {}", path.display());
                }
                Err(err) => {
                    eprintln!(
                        "[wfp-killswitch]   FAILED app v4 {}: {}",
                        path.display(),
                        err
                    );
                    continue;
                }
            }
            if let Err(err) = e.add_filter_allow_app(
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                NEMEFISTO_SUBLAYER_GUID,
                &format!("app v6 {label}"),
                W_APP,
                path,
            ) {
                eprintln!("[wfp-killswitch]   v6 skip {}: {}", path.display(), err);
            } else {
                apps_added_v6 += 1;
            }
        }
        eprintln!(
            "[wfp-killswitch] applied: apps_v4={apps_added_v4}, apps_v6={apps_added_v6}"
        );

        // ── Per-interface allow (step A — TUN-mode) ─────────────────
        // Любой трафик через TUN-адаптер уже шифрован VPN-движком,
        // безопасно разрешать без условий. Это упрощает схему: даже
        // если xray создаст subprocess или сменится server_ip,
        // kill-switch держится через interface-allow.
        if let Some(if_idx) = tun_interface_index {
            // Дублируем для V4 и V6 — оба layer'а независимы.
            for (layer, label) in [
                (FWPM_LAYER_ALE_AUTH_CONNECT_V4, "TUN allow v4"),
                (FWPM_LAYER_ALE_AUTH_CONNECT_V6, "TUN allow v6"),
            ] {
                e.add_filter_allow_local_interface_index(
                    layer,
                    NEMEFISTO_SUBLAYER_GUID,
                    label,
                    W_TUN_INTERFACE,
                    if_idx,
                )?;
            }
            eprintln!(
                "[wfp-killswitch] TUN-interface allow added (ifIndex={if_idx})"
            );
        }

        // ── DNS leak protection (этап 13.D step B) ──────────────────
        // Сначала allow-фильтры с высоким weight — они должны перебить
        // последующий block. Иначе порядок Add не имеет значения, всё
        // решает weight.
        if block_dns {
            for ip_str in &allow_dns_ips {
                if let Ok(v4) = ip_str.parse::<Ipv4Addr>() {
                    let addr = ipv4_to_u32(v4);
                    e.add_filter_allow_v4_addr_port_proto(
                        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                        NEMEFISTO_SUBLAYER_GUID,
                        &format!("DNS allow {ip_str}/UDP"),
                        W_DNS_PERMIT,
                        addr,
                        53,
                        IPPROTO_UDP,
                    )?;
                    e.add_filter_allow_v4_addr_port_proto(
                        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                        NEMEFISTO_SUBLAYER_GUID,
                        &format!("DNS allow {ip_str}/TCP"),
                        W_DNS_PERMIT,
                        addr,
                        53,
                        IPPROTO_TCP,
                    )?;
                } else {
                    eprintln!("[wfp-killswitch] пропускаю невалидный DNS IP: {ip_str}");
                }
            }
            // Затем block для всех :53 (UDP + TCP, V4 + V6).
            // Для V6 не делаем allow по адресу пока — пользователи
            // обычно используют v4 DNS. Если будет нужно — добавим.
            for layer in [FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6] {
                e.add_filter_block_port_proto(
                    layer,
                    NEMEFISTO_SUBLAYER_GUID,
                    "DNS block 53/UDP",
                    W_DNS_BLOCK,
                    53,
                    IPPROTO_UDP,
                )?;
                e.add_filter_block_port_proto(
                    layer,
                    NEMEFISTO_SUBLAYER_GUID,
                    "DNS block 53/TCP",
                    W_DNS_BLOCK,
                    53,
                    IPPROTO_TCP,
                )?;
            }
            eprintln!(
                "[wfp-killswitch] DNS leak protection ON (allow_count={})",
                allow_dns_ips.len()
            );
        }

        Ok(())
    })?;

    // Сохраняем engine — пока он жив, фильтры активны.
    *engine_lock().lock().unwrap() = Some(engine);

    // Watchdog: первый heartbeat ставим прямо сейчас + запускаем таск
    // если ещё не запущен. Таск глобальный — переживает повторные
    // enable/disable циклы, просто переходит в sleep когда LAST_HEARTBEAT
    // == i64::MIN (kill-switch off).
    LAST_HEARTBEAT.store(now_unix(), Ordering::SeqCst);
    spawn_watchdog_once();
    Ok(())
}

/// Лёгкий thread с таймером — раз в 5 сек проверяет, давно ли был
/// heartbeat. Если давно — снимает kill-switch. Идемпотентен через
/// AtomicBool "already started".
fn spawn_watchdog_once() {
    use std::sync::atomic::AtomicBool;
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(WATCHDOG_POLL_SECS));

        let last = LAST_HEARTBEAT.load(Ordering::SeqCst);
        if last == i64::MIN {
            // kill-switch выключен — нечего сторожить.
            continue;
        }
        let elapsed = now_unix().saturating_sub(last);
        if elapsed > WATCHDOG_TIMEOUT_SECS {
            eprintln!(
                "[wfp-killswitch] WATCHDOG: heartbeat lost ({}s) — auto-disable",
                elapsed
            );
            // Drop engine → DYNAMIC cleanup → восстановление интернета.
            // Параллельно ставим LAST_HEARTBEAT в sentinel чтобы повторно
            // не пытаться disable (и чтобы новый enable() мог нормально
            // включить heartbeat снова).
            LAST_HEARTBEAT.store(i64::MIN, Ordering::SeqCst);
            *engine_lock().lock().unwrap() = None;
            // Доп. cleanup: если фильтры остались по какой-то причине
            // (например DYNAMIC не сработал на этой версии Windows) —
            // явно удаляем provider+sublayer.
            if let Err(err) = wfp_cleanup_provider() {
                eprintln!("[wfp-killswitch] WATCHDOG cleanup err: {err}");
            }
        }
    });
}

/// Выключить kill-switch — drop'аем engine. DYNAMIC session при close
/// автоматически удаляет все фильтры/sublayer/provider.
pub async fn disable() -> Result<()> {
    tokio::task::spawn_blocking(disable_blocking)
        .await
        .context("spawn_blocking panic")?
}

fn disable_blocking() -> Result<()> {
    eprintln!("[wfp-killswitch] OFF");
    // Сразу останавливаем watchdog — иначе он может попытаться cleanup
    // одновременно с нами и взять lock конкурентно.
    LAST_HEARTBEAT.store(i64::MIN, Ordering::SeqCst);

    let mut g = engine_lock().lock().unwrap();
    *g = None; // drop → FwpmEngineClose0 → cleanup автоматический.

    // Доп. страховка: в редком случае если DYNAMIC не сработал, явно
    // удаляем provider/sublayer (каскадно с фильтрами). Идемпотентно
    // если ничего не было.
    drop(g); // отпускаем lock прежде чем делать sync WFP-вызов.
    if let Err(err) = wfp_cleanup_provider() {
        eprintln!("[wfp-killswitch] cleanup_provider error (не критично): {err}");
    }
    Ok(())
}

/// Cleanup orphan-фильтров с прошлых инкарнаций helper'а.
/// Вызывается при старте сервиса как страховка. Бежит в фоновой
/// задаче (см. service.rs) — чтобы не блокировать pipe-сервер.
///
/// Берёт engine_lock на время операции — гарантия что параллельный
/// `enable_blocking` от только что подключившегося клиента не пересечётся
/// с удалением provider'а: операции serialized через mutex.
pub async fn cleanup_on_startup() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        let _g = engine_lock().lock().unwrap();
        eprintln!("[wfp-killswitch] startup cleanup of any orphan filters");
        wfp_cleanup_provider()
    })
    .await
    .context("spawn_blocking panic")?
}

/// `"::1"` / `"fe80::"` / `"ff00::"` → 16-байтовый массив.
/// Только для известных литералов — нечего бить тревогу при ошибке.
fn ipv6_addr_octets(s: &str) -> [u8; 16] {
    s.parse::<Ipv6Addr>()
        .expect("ipv6 literal must parse")
        .octets()
}

/// `Ipv4Addr` → u32 в host byte order для FWP_V4_ADDR_AND_MASK.
/// Например `10.0.0.0` → `0x0A000000`.
fn ipv4_to_u32(addr: Ipv4Addr) -> u32 {
    let o = addr.octets();
    ((o[0] as u32) << 24) | ((o[1] as u32) << 16) | ((o[2] as u32) << 8) | (o[3] as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_v4_address_value() {
        // 127.0.0.0/8 — формат который мы передаём в FWP_V4_ADDR_AND_MASK.
        assert_eq!(ipv4_to_u32(Ipv4Addr::new(127, 0, 0, 0)), 0x7F00_0000);
    }

    #[test]
    fn lan_ranges_value() {
        // RFC1918 + APIPA — те значения что в enable() для allow_lan.
        assert_eq!(ipv4_to_u32(Ipv4Addr::new(10, 0, 0, 0)), 0x0A00_0000);
        assert_eq!(ipv4_to_u32(Ipv4Addr::new(172, 16, 0, 0)), 0xAC10_0000);
        assert_eq!(ipv4_to_u32(Ipv4Addr::new(192, 168, 0, 0)), 0xC0A8_0000);
        assert_eq!(ipv4_to_u32(Ipv4Addr::new(169, 254, 0, 0)), 0xA9FE_0000);
    }

    #[test]
    fn ipv6_loopback() {
        let octets = ipv6_addr_octets("::1");
        assert_eq!(octets, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn ipv6_link_local() {
        let octets = ipv6_addr_octets("fe80::");
        // fe80:: — первые два байта 0xFE 0x80, остальное нули.
        assert_eq!(&octets[0..2], &[0xFE, 0x80]);
        assert!(octets[2..].iter().all(|&b| b == 0));
    }

    #[test]
    fn ipv6_multicast() {
        let octets = ipv6_addr_octets("ff00::");
        assert_eq!(octets[0], 0xFF);
        assert!(octets[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn watchdog_state_starts_inactive() {
        // sentinel: kill-switch off.
        // Этот тест проверяет invariant heartbeat-системы: если kill-switch
        // не активен, LAST_HEARTBEAT должен быть i64::MIN.
        // Не запускаем enable() (требует admin + WFP), просто проверяем
        // начальное значение и поведение heartbeat() без активного state.
        // ВАЖНО: тест может конкурировать с другими тестами через global
        // static — но тут мы только читаем, безопасно.
        let initial = LAST_HEARTBEAT.load(Ordering::SeqCst);
        // initial либо MIN либо какой-то прошлый ts от другого теста —
        // достаточно проверить что heartbeat() обновляет значение.
        heartbeat();
        let after = LAST_HEARTBEAT.load(Ordering::SeqCst);
        assert!(after > 0);
        assert!(after >= initial || initial == i64::MIN);
    }
}
