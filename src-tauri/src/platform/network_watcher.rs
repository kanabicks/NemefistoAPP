//! Watcher смены сетевого окружения (этап 6.C + 13.M).
//!
//! Polling-таск раз в 5 секунд читает:
//! - имя интерфейса с минимальной метрикой default-route;
//! - имя Wi-Fi сети (SSID) если активный интерфейс — Wi-Fi.
//!
//! При изменении интерфейса emit-ит `network-changed`, при изменении
//! SSID — `wifi-changed`.
//!
//! **`network-changed`** — фронт делает reconnect VPN если он был
//! активен (маршруты Xray привязаны к старому интерфейсу).
//! **`wifi-changed`** — фронт проверяет trusted SSID list (13.M):
//! если новая сеть в whitelist — может выключить VPN или переключить
//! в `direct` режим. Удобно для дома/офиса где VPN не нужен.
//!
//! Polling выбран вместо `NotifyAddrChange` / `INetworkListManager`
//! потому что он проще, кроссплатформенен (для будущего портирования)
//! и cost ничтожный — `GetIpForwardTable2` + `netsh` отрабатывают за
//! ~50 мс суммарно.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{async_runtime, AppHandle, Emitter};

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

/// Payload события `wifi-changed` (этап 13.M).
#[derive(Serialize, Clone)]
pub struct WifiChange {
    /// Прежний SSID. None — не было Wi-Fi или адаптер был disconnected.
    pub from: Option<String>,
    /// Текущий SSID. None — Wi-Fi отключён / отсутствует / Ethernet.
    pub to: Option<String>,
}

/// Запустить watcher как фоновую задачу через рантайм Tauri.
///
/// **ВАЖНО**: используем `tauri::async_runtime::spawn`, а не голый
/// `tokio::spawn`. В Tauri 2 setup-callback выполняется ДО того как
/// контекст Tokio runtime становится доступным внутри setup, поэтому
/// прямой `tokio::spawn` паникует с `there is no reactor running`.
/// `async_runtime::spawn` сам подхватывает managed-runtime Tauri.
///
/// Идемпотентно: повторные вызовы — no-op.
pub fn start(app: AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    async_runtime::spawn(async move {
        let mut last_iface = get_default_route_interface_name();
        let mut last_ssid = read_current_ssid().await;

        // Эмитим начальный SSID сразу — фронт нуждается в нём чтобы
        // проверить «попали ли мы в trusted Wi-Fi» уже на старте,
        // не ждать первой смены сети.
        if last_ssid.is_some() {
            let payload = WifiChange {
                from: None,
                to: last_ssid.clone(),
            };
            let _ = app.emit("wifi-changed", &payload);
        }

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let current_iface = get_default_route_interface_name();
            if current_iface != last_iface {
                let payload = NetworkChange {
                    from: last_iface.clone(),
                    to: current_iface.clone(),
                };
                eprintln!(
                    "[network-watcher] смена интерфейса: {:?} → {:?}",
                    payload.from, payload.to
                );
                let _ = app.emit("network-changed", &payload);
                last_iface = current_iface;
            }

            let current_ssid = read_current_ssid().await;
            if current_ssid != last_ssid {
                let payload = WifiChange {
                    from: last_ssid.clone(),
                    to: current_ssid.clone(),
                };
                eprintln!(
                    "[network-watcher] смена ssid: {:?} → {:?}",
                    payload.from, payload.to
                );
                let _ = app.emit("wifi-changed", &payload);
                last_ssid = current_ssid;
            }
        }
    });
}

/// Прочитать SSID активного Wi-Fi через `netsh wlan show interfaces`.
///
/// Возвращает `None` если:
/// - адаптера Wi-Fi нет физически;
/// - адаптер выключен / disconnected;
/// - команда `netsh` упала (на Server-сборках Windows без WlanAPI).
///
/// Парсинг толерантен к локализации: ловим строку `SSID  : <name>`
/// (пробелы вокруг двоеточия любые) и берём первое совпадение —
/// `BSSID` идёт после, так что not first.
#[cfg(windows)]
async fn read_current_ssid() -> Option<String> {
    use tokio::process::Command;
    // 0x08000000 = CREATE_NO_WINDOW — без мигающего консольного окна.
    // tokio::process::Command реэкспортирует Windows-методы из
    // `std::os::windows::process::CommandExt` без явного импорта.
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let out = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // netsh печатает на cp866/cp1251 в русской локали. Для нашего
    // парсинга важна ASCII-часть (`SSID`, `:`, имя сети). Имя
    // обычно UTF-8 / ASCII, но если кириллическое — может побиться.
    // Тут приемлемо: пользователь и так введёт имя руками в UI,
    // а сравнение строк точное.
    let text = String::from_utf8_lossy(&out.stdout);
    parse_ssid(&text)
}

#[cfg(not(windows))]
async fn read_current_ssid() -> Option<String> {
    None
}

/// Парсер вывода `netsh wlan show interfaces`. Вынесен отдельно
/// чтобы тестировался без зависимости от Windows API.
fn parse_ssid(text: &str) -> Option<String> {
    // Также проверяем State: connected/Подключено — иначе SSID может
    // быть пустой строкой или старый.
    let mut connected = false;
    let mut ssid: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            // State / Состояние : connected / Подключено.
            // `to_lowercase()` — Unicode-aware (умеет с кириллицей),
            // в отличие от `to_ascii_lowercase` который её игнорирует.
            let key_lower = key.to_lowercase();
            if key_lower == "state" || key_lower == "состояние" {
                let v = value.to_lowercase();
                if v == "connected" || v == "подключено" {
                    connected = true;
                }
            }
            // SSID отдельно от BSSID. Берём первое совпадение точно
            // равное "SSID" чтобы не зацепить BSSID или SSID-расширения.
            if key_lower == "ssid" && ssid.is_none() && !value.is_empty() {
                ssid = Some(value.to_string());
            }
        }
    }

    if connected {
        ssid
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::parse_ssid;

    #[test]
    fn parses_english_output() {
        let s = "
There is 1 interface on the system:

    Name                   : Wi-Fi
    Description            : Intel Wi-Fi 6
    GUID                   : 00000000-0000-0000-0000-000000000000
    State                  : connected
    SSID                   : MyHomeWifi
    BSSID                  : aa:bb:cc:dd:ee:ff
    Network type           : Infrastructure
";
        assert_eq!(parse_ssid(s), Some("MyHomeWifi".into()));
    }

    #[test]
    fn parses_russian_output() {
        let s = "
В системе доступен 1 интерфейс:

    Имя                    : Wi-Fi
    Описание               : Intel Wi-Fi 6
    Состояние              : Подключено
    SSID                   : DomashniyWiFi
    BSSID                  : aa:bb:cc:dd:ee:ff
";
        assert_eq!(parse_ssid(s), Some("DomashniyWiFi".into()));
    }

    #[test]
    fn returns_none_when_disconnected() {
        let s = "
    State                  : disconnected
    SSID                   : OldName
";
        assert_eq!(parse_ssid(s), None);
    }

    #[test]
    fn returns_none_when_no_ssid() {
        let s = "
    State                  : connected
";
        assert_eq!(parse_ssid(s), None);
    }
}
