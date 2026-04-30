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
