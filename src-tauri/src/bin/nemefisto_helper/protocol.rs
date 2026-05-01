//! JSON-RPC протокол между helper-сервисом и Tauri-приложением.
//!
//! Каждое сообщение — одна строка JSON, заканчивается `\n`. Helper читает
//! строку, парсит как `Request`, выполняет, отвечает `Response` (тоже одна
//! строка JSON + `\n`). Соединение остаётся открытым — клиент может слать
//! несколько команд подряд.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Health-check. Helper отвечает `Response::Pong`.
    Ping,
    /// Версия helper-а.
    Version,
    /// Запустить tun2socks + сконфигурировать routing.
    /// `socks_port` — наш Xray-SOCKS5 inbound на 127.0.0.1.
    /// `server_host` — хост или IP внешнего VPN-сервера (резолвится для bypass-route).
    /// `dns` — DNS-сервер, который выставится на TUN-интерфейс.
    /// `tun2socks_path` — абсолютный путь к tun2socks-x86_64-pc-windows-msvc.exe
    /// (helper не знает где лежит binaries-папка Tauri-приложения).
    TunStart {
        socks_port: u16,
        server_host: String,
        dns: String,
        tun2socks_path: String,
        /// SOCKS5 auth (этап 9.G): tun2socks использует
        /// `socks5://user:pass@host:port` если оба заданы. Иначе noauth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socks_username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socks_password: Option<String>,
        /// Маскировка TUN-имени (этап 12.E): если задано, helper создаёт
        /// адаптер с этим именем. Иначе — стандартный `nemefisto-<pid>`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tun_name_override: Option<String>,
    },
    /// Остановить tun2socks и откатить добавленные routes.
    TunStop,
    /// Включить kill switch (этап 13.D — настоящий WFP).
    ///
    /// `server_ips` — список IP-адресов VPN-сервера (Tauri-main делает
    /// DNS-резолв перед вызовом, потому что после включения kill-switch'а
    /// DNS-запросы вне VPN заблокированы).
    ///
    /// `allow_lan` — пускать ли локальную сеть (10/8, 172.16/12,
    /// 192.168/16, 169.254/16, fe80::/10, ff00::/8).
    ///
    /// `allow_app_paths` — абсолютные пути к нашим бинарям, которым
    /// разрешён исходящий трафик (xray.exe, mihomo.exe, tun2socks.exe).
    /// Без этого VPN-движок не сможет соединиться даже если IP сервера
    /// есть в server_ips.
    KillSwitchEnable {
        #[serde(default)]
        server_ips: Vec<String>,
        #[serde(default)]
        allow_lan: bool,
        #[serde(default)]
        allow_app_paths: Vec<String>,
        /// DNS leak protection (этап 13.D step B): блокировать весь
        /// :53/UDP+TCP трафик кроме явно разрешённых IP.
        #[serde(default)]
        block_dns: bool,
        /// IPv4 адреса VPN-DNS которые остаются разрешены при `block_dns`.
        /// В TUN-mode обычно [`198.18.0.1`] (наш TUN gateway).
        #[serde(default)]
        allow_dns_ips: Vec<String>,
        /// 13.S strict mode: НЕ давать общий allow_app для VPN-движков
        /// (xray/mihomo). Они смогут соединяться только на server_ips
        /// (через add_filter_allow_v4_addr_port_proto, который добавляется
        /// в любом случае). Direct outbound xray по `geosite:ru` будет
        /// блокирован — это и есть смысл strict mode.
        #[serde(default)]
        strict_mode: bool,
    },
    /// Выключить kill switch — drop'ает WFP DYNAMIC engine,
    /// все наши фильтры удаляются автоматически.
    KillSwitchDisable,
    /// Heartbeat для kill-switch watchdog (этап 13.D).
    /// Tauri-main шлёт каждые ~20 сек пока активен kill-switch. Если
    /// helper не получит heartbeat 60+ секунд — фильтры автоматически
    /// снимаются (страховка от зависания main-процесса).
    KillSwitchHeartbeat,
    /// Emergency cleanup всех WFP-фильтров с нашим provider GUID.
    /// Используется UI-кнопкой «аварийный сброс» — даже если main
    /// сейчас не имеет активного kill-switch state, удалит всё что
    /// потенциально зависло от прошлых сессий.
    KillSwitchForceCleanup,
    /// Cleanup orphan TUN-адаптеров (`nemefisto-*`) и half-default
    /// routes через `198.18.0.1`. Используется UI-кнопкой
    /// «восстановить сеть» когда видимо, что что-то осталось от
    /// упавшей сессии. Безопасно вызывать только когда VPN не активен.
    OrphanCleanup,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Pong,
    Version { version: String },
    /// Успешный результат операции без полезной нагрузки.
    Ok,
    /// Ошибка с описанием.
    Error { message: String },
}

impl Response {
    pub fn err(msg: impl Into<String>) -> Self {
        Self::Error { message: msg.into() }
    }
}

pub const PIPE_NAME: &str = r"\\.\pipe\nemefisto-helper";
pub const SERVICE_NAME: &str = "NemefistoHelper";
pub const SERVICE_DISPLAY_NAME: &str = "Nemefisto VPN Helper";
pub const SERVICE_DESCRIPTION: &str = "Управление TUN-интерфейсом и системной маршрутизацией для Nemefisto VPN.";
