//! Watcher смены сетевого окружения (этап 6.C).
//!
//! Polling-таск раз в 5 секунд читает имя интерфейса с минимальной
//! метрикой default-route. При изменении (например, ноутбук
//! переключился с Ethernet на Wi-Fi или поменял Wi-Fi сеть) emit-ит
//! событие `network-changed` во фронтенд через Tauri Event API.
//!
//! Фронт ловит это событие и, если VPN был активен, делает reconnect —
//! иначе при смене сети маршруты Xray направляются через старый
//! интерфейс и трафик не доходит.
//!
//! Polling выбран вместо `NotifyAddrChange` / `INetworkListManager`
//! потому что он проще, кроссплатформенен (для будущего портирования)
//! и cost ничтожный — `GetIpForwardTable2` отрабатывает за <1 мс.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::network::get_default_route_interface_name;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Флаг что watcher уже запущен — защита от двойного старта в случае
/// если Tauri setup вызовется несколько раз.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Payload события `network-changed` для фронта.
#[derive(Serialize, Clone)]
pub struct NetworkChange {
    /// Прежний интерфейс (None — был disconnected).
    pub from: Option<String>,
    /// Новый интерфейс (None — стал disconnected).
    pub to: Option<String>,
}

/// Запустить watcher как фоновую tokio-задачу.
///
/// Идемпотентно: повторные вызовы — no-op.
pub fn start(app: AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    tokio::spawn(async move {
        let mut last = get_default_route_interface_name();
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let current = get_default_route_interface_name();
            if current != last {
                let payload = NetworkChange {
                    from: last.clone(),
                    to: current.clone(),
                };
                eprintln!(
                    "[network-watcher] смена интерфейса: {:?} → {:?}",
                    payload.from, payload.to
                );
                let _ = app.emit("network-changed", &payload);
                last = current;
            }
        }
    });
}
