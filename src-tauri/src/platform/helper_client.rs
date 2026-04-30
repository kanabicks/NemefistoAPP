//! RPC-клиент для подключения к helper-сервису через named pipe.
//!
//! Помещает каждое JSON-сообщение строкой с `\n`-терминатором, как в pipe.rs
//! на стороне сервиса. Каждый вызов открывает свежее подключение, шлёт один
//! request и закрывает. Helper-pipe.rs умеет много клиентов — каждый
//! обработчик в отдельной задаче.
//!
//! ВАЖНО: типы должны точно совпадать с тегами в
//! `src/bin/nemefisto_helper/protocol.rs`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ClientOptions;

const PIPE_NAME: &str = r"\\.\pipe\nemefisto-helper";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum HelperRequest {
    Ping,
    Version,
    TunStart {
        socks_port: u16,
        server_host: String,
        dns: String,
        tun2socks_path: String,
        /// SOCKS5 auth (этап 9.G): если задан, tun2socks подключится с
        /// учётными данными `socks5://user:pass@127.0.0.1:port`. Xray
        /// принимает auth: password только когда обе строки заданы.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socks_username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        socks_password: Option<String>,
        /// Маскировка TUN-имени (этап 12.E): если задано — helper создаёт
        /// адаптер с этим именем вместо стандартного `nemefisto-<pid>`.
        /// UI должен сгенерировать имя случайным образом, чтобы оно было
        /// разным от запуска к запуску.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tun_name_override: Option<String>,
    },
    TunStop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum HelperResponse {
    Pong,
    Version { version: String },
    Ok,
    Error { message: String },
}

/// Открыть pipe с retry — сервис может быть busy сразу после старта или
/// перезапуска. Возвращает первый успешный клиент за 1 секунду или ошибку.
async fn open_pipe() -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..10 {
        match ClientOptions::new().open(PIPE_NAME) {
            Ok(client) => return Ok(client),
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    let err = last_err.map(|e| format!("{e}"))
        .unwrap_or_else(|| "не удалось открыть pipe".into());
    bail!("helper-сервис недоступен ({PIPE_NAME}): {err}")
}

/// Низкоуровневый round-trip: отправить request, получить response.
pub async fn send(req: HelperRequest) -> Result<HelperResponse> {
    let client = open_pipe().await?;
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut reader = BufReader::new(read_half);

    let mut payload = serde_json::to_vec(&req)?;
    payload.push(b'\n');
    write_half.write_all(&payload).await.context("запись в pipe")?;
    write_half.flush().await.ok();

    let mut response_line = String::new();
    let n = reader.read_line(&mut response_line).await.context("чтение из pipe")?;
    if n == 0 {
        bail!("helper закрыл соединение без ответа");
    }
    let resp: HelperResponse = serde_json::from_str(response_line.trim())
        .with_context(|| format!("невалидный JSON-ответ: {response_line:?}"))?;
    Ok(resp)
}

/// Health-check. Bool на успех / Result для UI «статус helper-а».
pub async fn ping() -> Result<()> {
    match send(HelperRequest::Ping).await? {
        HelperResponse::Pong => Ok(()),
        HelperResponse::Error { message } => bail!("helper: {message}"),
        other => bail!("ожидали Pong, получили {other:?}"),
    }
}

/// Поднять TUN-режим: helper запустит tun2socks, добавит маршруты,
/// настроит DNS на TUN-интерфейсе.
///
/// Опционально:
/// - `socks_username` / `socks_password` — для SOCKS5 auth (9.G);
///   передаётся в tun2socks как `socks5://user:pass@host:port`.
/// - `tun_name_override` — кастомное имя TUN-адаптера для маскировки
///   от детекта приложений по имени интерфейса (12.E).
pub async fn tun_start(
    socks_port: u16,
    server_host: String,
    dns: String,
    tun2socks_path: String,
    socks_username: Option<String>,
    socks_password: Option<String>,
    tun_name_override: Option<String>,
) -> Result<()> {
    let resp = send(HelperRequest::TunStart {
        socks_port,
        server_host,
        dns,
        tun2socks_path,
        socks_username,
        socks_password,
        tun_name_override,
    })
    .await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// Остановить TUN-режим. Идемпотентно: если TUN не был активен, helper
/// вернёт error «TUN-режим не запущен» — мы тихо игнорируем.
pub async fn tun_stop() -> Result<()> {
    let resp = send(HelperRequest::TunStop).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        // Игнорируем «не запущен» — нормальное состояние при disconnect
        // в proxy-режиме или повторном вызове.
        HelperResponse::Error { message } if message.contains("не запущен") => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}
