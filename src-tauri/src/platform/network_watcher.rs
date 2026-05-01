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
///
/// 0.1.1 / Bug 2: запускаем netsh через `cmd /c chcp 65001 && netsh ...`
/// — без этого на русской Windows вывод приходит в cp866 (DOS)
/// либо cp1251 (Windows), `from_utf8_lossy` коверкает кириллицу в
/// `?`-знаки, и парсер не находит ни «состояние» ни кириллический
/// SSID. С `chcp 65001` netsh печатает UTF-8 → парсер видит
/// исходные строки и работает на любой локали.
#[cfg(windows)]
async fn read_current_ssid() -> Option<String> {
    use tokio::process::Command;
    // 0x08000000 = CREATE_NO_WINDOW — без мигающего консольного окна.
    // tokio::process::Command реэкспортирует Windows-методы из
    // `std::os::windows::process::CommandExt` без явного импорта.
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // Сначала пробуем с UTF-8 chcp — даст корректные кириллические SSID.
    let out = Command::new("cmd")
        .args(["/c", "chcp 65001 >nul && netsh wlan show interfaces"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_ssid(&text)
}

#[cfg(not(windows))]
async fn read_current_ssid() -> Option<String> {
    None
}

/// Парсер вывода `netsh wlan show interfaces`. Вынесен отдельно
/// чтобы тестировался без зависимости от Windows API.
///
/// 0.1.1 / Bug 2: убрана зависимость от строки `State: connected`.
/// Раньше мы требовали явного «connected/подключено» в выводе, что
/// ломалось на:
///   - не-RU/EN локалях (немецкий «Verbunden», француский «connecté»),
///   - кодировках где chcp коверкал кириллицу до `?`,
///   - случаях когда строки State в выводе вообще не было (старый
///     адаптер).
///
/// Новая логика: netsh выводит непустую `SSID:` строку **только**
/// для подключённых адаптеров. Если нашли непустой SSID — он
/// активный. Это надёжнее и проще.
fn parse_ssid(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        // Точное равенство «SSID» (case-insensitive) — чтобы не
        // зацепить BSSID, MAC адрес или SSID-расширения. Это
        // ASCII-only ключ, на всех локалях остаётся «SSID».
        if key.eq_ignore_ascii_case("ssid") {
            return Some(value.to_string());
        }
    }
    None
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

    /// 0.1.1 / Bug 2: парсер должен работать на немецкой локали без
    /// доп. изменений (раньше падал из-за «Verbunden» != «connected»).
    #[test]
    fn parses_german_output() {
        let s = "
    Name                   : WLAN
    Status                 : Verbunden
    SSID                   : MeinNetzwerk
    BSSID                  : 11:22:33:44:55:66
";
        assert_eq!(parse_ssid(s), Some("MeinNetzwerk".into()));
    }

    /// netsh не печатает SSID для disconnected адаптеров — наш парсер
    /// корректно вернёт None просто потому что строки нет.
    #[test]
    fn returns_none_when_no_ssid_at_all() {
        let s = "
    State                  : disconnected
";
        assert_eq!(parse_ssid(s), None);
    }

    /// BSSID содержит «SSID» как подстроку, но точное равенство ключа
    /// (case-insensitive ASCII) исключает совпадение.
    #[test]
    fn does_not_match_bssid() {
        let s = "
    BSSID                  : aa:bb:cc:dd:ee:ff
";
        assert_eq!(parse_ssid(s), None);
    }

    /// SSID с emoji / кириллицей читается как есть после chcp 65001.
    #[test]
    fn parses_unicode_ssid() {
        let s = "
    SSID                   : Дом 🏠 Wi-Fi
";
        assert_eq!(parse_ssid(s), Some("Дом 🏠 Wi-Fi".into()));
    }
}
