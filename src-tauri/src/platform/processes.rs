//! 9.B — Детект запущенных сторонних VPN-клиентов.
//!
//! Перебираем все процессы через `EnumProcesses` + `K32GetModuleBaseNameW`.
//! Сравниваем имена с whitelist'ом известных VPN-клиентов и возвращаем
//! human-readable названия найденных. Не блокирует connect — это
//! предупреждающий банер.
//!
//! Не требует admin-прав: `PROCESS_QUERY_LIMITED_INFORMATION` доступен
//! с Vista+ для любого процесса (имена exe — не sensitive-данные).

#[cfg(windows)]
pub fn detect_competing_vpns() -> Vec<String> {
    use std::collections::HashSet;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::ProcessStatus::{EnumProcesses, K32GetModuleBaseNameW};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // exe-имя (lowercase, без .exe в самой строке для устойчивости) → human-readable.
    // Проверка делается по `.starts_with(...)` после strip-а ".exe", чтобы
    // ловить вариации вроде "Happ.exe", "happ.exe", "HappPro.exe".
    const KNOWN: &[(&str, &str)] = &[
        ("happ", "Happ"),
        ("outlineclient", "Outline Client"),
        ("outline-client", "Outline Client"),
        ("openvpngui", "OpenVPN GUI"),
        ("openvpn", "OpenVPN"),
        ("wireguard", "WireGuard"),
        ("nordvpn", "NordVPN"),
        ("expressvpn", "ExpressVPN"),
        ("protonvpn", "ProtonVPN"),
        ("mullvad-gui", "Mullvad VPN"),
        ("mullvad-daemon", "Mullvad VPN"),
        ("mullvad", "Mullvad VPN"),
        ("v2rayn", "v2rayN"),
        ("v2rayng", "v2rayNG"),
        ("clash-verge", "Clash Verge"),
        ("clashx", "ClashX"),
        ("clashw", "ClashW"),
        ("clash", "Clash"),
        ("hiddifynext", "Hiddify Next"),
        ("hiddify", "Hiddify"),
        ("furious", "Furious"),
        ("nekoray", "Nekoray"),
        ("nekobox", "Nekobox"),
        ("amneziavpn", "AmneziaVPN"),
        ("windscribe", "Windscribe"),
        ("shadowsocks", "Shadowsocks"),
        ("singbox", "sing-box"),
        ("sing-box", "sing-box"),
        ("ss-local", "Shadowsocks"),
        ("incy", "INCY"),
        ("v2raytun", "v2rayTun"),
    ];

    // Самоисключение — наш собственный exe не считаем конкурентом.
    const OUR_EXES: &[&str] = &["vpn-client", "nemefisto-helper", "xray", "mihomo", "tun2socks"];

    unsafe {
        let mut pids = vec![0u32; 4096];
        let mut bytes_returned: u32 = 0;
        let cb = (pids.len() * std::mem::size_of::<u32>()) as u32;
        if EnumProcesses(pids.as_mut_ptr(), cb, &mut bytes_returned) == 0 {
            return Vec::new();
        }
        let count = (bytes_returned as usize) / std::mem::size_of::<u32>();
        pids.truncate(count);

        let mut found: HashSet<&'static str> = HashSet::new();

        for &pid in &pids {
            if pid == 0 {
                continue;
            }
            let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if h.is_null() {
                continue;
            }

            let mut name_buf = [0u16; 260];
            let len = K32GetModuleBaseNameW(
                h,
                std::ptr::null_mut(),
                name_buf.as_mut_ptr(),
                name_buf.len() as u32,
            );
            CloseHandle(h);

            if len == 0 {
                continue;
            }
            let name = String::from_utf16_lossy(&name_buf[..len as usize]).to_lowercase();
            // Нормализуем: режем расширение для устойчивости сравнения.
            let stem = name.strip_suffix(".exe").unwrap_or(&name);

            // Самоисключение.
            if OUR_EXES.iter().any(|own| stem == *own) {
                continue;
            }

            for (key, label) in KNOWN {
                if stem == *key || stem.starts_with(key) {
                    found.insert(*label);
                    break;
                }
            }
        }

        let mut out: Vec<String> = found.into_iter().map(String::from).collect();
        out.sort();
        out
    }
}

#[cfg(not(windows))]
pub fn detect_competing_vpns() -> Vec<String> {
    Vec::new()
}
