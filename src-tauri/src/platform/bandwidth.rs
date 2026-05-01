//! Измерение скорости передачи на default-route интерфейсе (этап 13.O).
//!
//! Раз в секунду читаем `GetIfTable2`, находим интерфейс соответствующий
//! текущему default-route (обычно Wi-Fi/Ethernet, либо наш TUN при
//! активном VPN — он перехватывает default-route). Считаем дельту
//! `InOctets`/`OutOctets` от предыдущего полла. Emit-им event
//! `bandwidth-tick` с `{ up_bps, down_bps }` (bytes per second).
//!
//! Используется floating-окном (13.O) и опционально главным окном
//! для отображения live-скорости. В proxy-режиме default-route смотрит
//! на физический интерфейс — так что метрика покажет общий системный
//! трафик (а не только VPN). В TUN-режиме default-route уходит в наш
//! WinTUN-адаптер — метрика покажет именно VPN-трафик.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{async_runtime, AppHandle, Emitter};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Защита от двойного старта (как в network_watcher).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Payload `bandwidth-tick`. Числа в байтах/сек, фронт сам форматирует
/// в KB/s / MB/s. `iface` опционально — если null, измерения нет
/// (default-route не определён, например только что после reboot).
#[derive(Serialize, Clone)]
pub struct BandwidthTick {
    pub up_bps: u64,
    pub down_bps: u64,
    pub iface: Option<String>,
}

pub fn start(app: AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    async_runtime::spawn(async move {
        // Предыдущие октеты + имя интерфейса. При смене интерфейса
        // дельту считать нельзя (счётчик у нового свой) — сбрасываем.
        let mut prev: Option<(String, u64, u64)> = None;
        // Каждые ~10 секунд печатаем диагностику в stderr (виден в
        // `tauri dev` консоли). Если bandwidth всегда нулевой, лог
        // покажет какой интерфейс мы мониторим и нашли ли его.
        let mut tick_count: u32 = 0;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            tick_count = tick_count.wrapping_add(1);

            let iface = super::network::get_default_route_interface_name();
            let counters = match iface.as_deref() {
                Some(name) => read_iface_octets(name),
                None => None,
            };

            if tick_count % 10 == 0 {
                eprintln!(
                    "[bandwidth] iface={:?} counters={:?} prev={:?}",
                    iface,
                    counters,
                    prev.as_ref().map(|(n, _, _)| n)
                );
            }

            let tick = match (counters, prev.as_ref()) {
                (Some((cur_in, cur_out)), Some((prev_name, prev_in, prev_out)))
                    if Some(prev_name.as_str()) == iface.as_deref() =>
                {
                    // Корректная дельта только если интерфейс тот же.
                    let down = cur_in.saturating_sub(*prev_in);
                    let up = cur_out.saturating_sub(*prev_out);
                    BandwidthTick {
                        up_bps: up,
                        down_bps: down,
                        iface: iface.clone(),
                    }
                }
                _ => {
                    // Первый замер либо смена интерфейса — отдаём 0/0.
                    BandwidthTick {
                        up_bps: 0,
                        down_bps: 0,
                        iface: iface.clone(),
                    }
                }
            };

            // Обновляем prev (или сбрасываем, если counters=None).
            prev = match (iface, counters) {
                (Some(name), Some((cin, cout))) => Some((name, cin, cout)),
                _ => None,
            };

            // Сбой эмита (например, главное окно ещё не готово) — игнор.
            let _ = app.emit("bandwidth-tick", &tick);
        }
    });
}

/// Прочитать `InOctets` / `OutOctets` для интерфейса по alias-имени.
///
/// Возвращает `None` если интерфейс не найден или GetIfTable2 упал.
#[cfg(windows)]
fn read_iface_octets(alias: &str) -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::NO_ERROR;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };

    unsafe {
        let mut tbl_ptr: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut tbl_ptr) != NO_ERROR || tbl_ptr.is_null() {
            return None;
        }
        let tbl = &*tbl_ptr;
        let rows = std::slice::from_raw_parts(tbl.Table.as_ptr(), tbl.NumEntries as usize);

        let mut result: Option<(u64, u64)> = None;
        for row in rows {
            // Alias — UTF-16, null-terminated.
            let alias_arr = &row.Alias;
            let len = alias_arr
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(alias_arr.len());
            let row_alias = String::from_utf16_lossy(&alias_arr[..len]);
            if row_alias == alias {
                result = Some((row.InOctets, row.OutOctets));
                break;
            }
        }
        FreeMibTable(tbl_ptr as *mut _);
        result
    }
}

#[cfg(not(windows))]
fn read_iface_octets(_alias: &str) -> Option<(u64, u64)> {
    None
}
