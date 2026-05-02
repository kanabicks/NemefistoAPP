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
    /// Включить kill switch (этап 13.D — настоящий WFP).
    /// `server_ips` — массив IP уже резолвленный в Tauri-main.
    /// `allow_lan` — пускать ли локальную сеть.
    /// `allow_app_paths` — пути к нашим бинарям (allowlist по app-id).
    KillSwitchEnable {
        #[serde(default)]
        server_ips: Vec<String>,
        #[serde(default)]
        allow_lan: bool,
        #[serde(default)]
        allow_app_paths: Vec<String>,
        /// DNS leak protection (13.D step B). См. protocol.rs.
        #[serde(default)]
        block_dns: bool,
        #[serde(default)]
        allow_dns_ips: Vec<String>,
        /// 13.S strict mode — без общего allow_app для xray/mihomo.
        #[serde(default)]
        strict_mode: bool,
    },
    KillSwitchDisable,
    /// Heartbeat для watchdog: главный шлёт каждые ~20 сек, иначе
    /// helper через 60+ сек снимет фильтры сам. См. firewall.rs.
    KillSwitchHeartbeat,
    /// Emergency cleanup — снять любые наши WFP-фильтры (для UI-кнопки
    /// «аварийный сброс»).
    KillSwitchForceCleanup,
    /// Cleanup orphan TUN-адаптеров (`nemefisto-*`) и half-default
    /// маршрутов через `198.18.0.1`. Часть UI-кнопки «восстановить сеть».
    OrphanCleanup,
    /// 14.E: read-only проверка остатков WFP-фильтров от прошлой
    /// сессии. Helper смотрит существование sublayer с нашим GUID.
    WfpQueryOrphan,
    /// 13.L: запустить mihomo как SYSTEM-процесс (для built-in TUN).
    MihomoStart {
        config_path: String,
        mihomo_exe_path: String,
        data_dir: String,
    },
    /// 13.L: остановить SYSTEM-spawned mihomo. Идемпотентно.
    MihomoStop,
    /// sing-box миграция (v7): запустить sing-box как SYSTEM-процесс
    /// для built-in TUN-режима (CreateAdapter WinTUN требует админа).
    SingBoxStart {
        config_path: String,
        singbox_exe_path: String,
        data_dir: String,
    },
    /// sing-box миграция (v7): остановить SYSTEM-spawned sing-box.
    /// Идемпотентно.
    SingBoxStop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum HelperResponse {
    Pong,
    Version {
        version: String,
        /// 0.1.2: версия wire-протокола helper'а. Старые helper'ы
        /// (v0.1.1 и раньше) не возвращают это поле — десериализуем
        /// в 0, что триггерит auto-reinstall в `helper_bootstrap`.
        #[serde(default)]
        protocol_version: u32,
    },
    Ok,
    /// 14.E: ответ на `WfpQueryOrphan`.
    WfpOrphan { has_orphan: bool },
    Error { message: String },
}

/// Минимально-поддерживаемая версия протокола. Если helper отвечает
/// меньшей — `helper_bootstrap` форсит uninstall+install. Бампается
/// синхронно с константой в `nemefisto_helper::protocol`.
pub const MIN_HELPER_PROTOCOL_VERSION: u32 = 8;

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

/// Получить версию helper-сервиса. Используется `helper_bootstrap` для
/// проверки совместимости wire-протокола: если helper старше нашего
/// `MIN_HELPER_PROTOCOL_VERSION` — форсим reinstall.
pub async fn version() -> Result<(String, u32)> {
    match send(HelperRequest::Version).await? {
        HelperResponse::Version {
            version,
            protocol_version,
        } => Ok((version, protocol_version)),
        HelperResponse::Error { message } => bail!("helper: {message}"),
        other => bail!("ожидали Version, получили {other:?}"),
    }
}

/// Поднять TUN-режим: helper запустит tun2socks, добавит маршруты,
/// настроит DNS на TUN-интерфейсе.
/// Включить kill switch — WFP-фильтры на уровне ядра блокируют весь
/// outbound кроме allowlist'а (этап 13.D).
///
/// - `server_ips` — IP-адреса VPN-сервера, уже резолвленные;
/// - `allow_lan` — пускать ли локальную сеть;
/// - `allow_app_paths` — абсолютные пути к VPN-движкам и tun2socks;
/// - `block_dns` — DNS-leak protection: блокировать весь :53 кроме
///   `allow_dns_ips` (13.D step B);
/// - `allow_dns_ips` — IPv4 адреса разрешённых DNS-серверов (когда
///   `block_dns=true`).
pub async fn kill_switch_enable(
    server_ips: Vec<String>,
    allow_lan: bool,
    allow_app_paths: Vec<String>,
    block_dns: bool,
    allow_dns_ips: Vec<String>,
    strict_mode: bool,
) -> Result<()> {
    let resp = send(HelperRequest::KillSwitchEnable {
        server_ips,
        allow_lan,
        allow_app_paths,
        block_dns,
        allow_dns_ips,
        strict_mode,
    })
    .await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// Выключить kill switch (восстановить default-allow). Идемпотентно.
pub async fn kill_switch_disable() -> Result<()> {
    let resp = send(HelperRequest::KillSwitchDisable).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// Heartbeat для kill-switch watchdog. Зовётся каждые ~20 сек пока
/// VPN активен. Если helper не получит ping 60+ сек — он автоматически
/// снимет фильтры (страховка от зависания main).
pub async fn kill_switch_heartbeat() -> Result<()> {
    let resp = send(HelperRequest::KillSwitchHeartbeat).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// Аварийный сброс — удалить все WFP-фильтры с нашим provider GUID.
/// Используется UI-кнопкой когда что-то пошло не так и интернет
/// заблокирован. Идемпотентно: если ничего нет — просто Ok.
pub async fn kill_switch_force_cleanup() -> Result<()> {
    let resp = send(HelperRequest::KillSwitchForceCleanup).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// 13.L: spawn mihomo как SYSTEM-процесс через helper. Используется
/// в built-in TUN-режиме где требуются админ-права на CreateAdapter.
pub async fn mihomo_start(
    config_path: String,
    mihomo_exe_path: String,
    data_dir: String,
) -> Result<()> {
    let resp = send(HelperRequest::MihomoStart {
        config_path,
        mihomo_exe_path,
        data_dir,
    })
    .await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// 13.L: остановить SYSTEM-spawned mihomo. Идемпотентно — если helper
/// не запускал mihomo, вернёт Ok сразу.
pub async fn mihomo_stop() -> Result<()> {
    let resp = send(HelperRequest::MihomoStop).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// sing-box миграция: spawn sing-box как SYSTEM-процесс через helper.
/// Используется в built-in TUN-режиме где требуются админ-права на
/// CreateAdapter. Семантически зеркалит `mihomo_start`.
pub async fn singbox_start(
    config_path: String,
    singbox_exe_path: String,
    data_dir: String,
) -> Result<()> {
    let resp = send(HelperRequest::SingBoxStart {
        config_path,
        singbox_exe_path,
        data_dir,
    })
    .await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// Остановить SYSTEM-spawned sing-box. Идемпотентно — если helper не
/// запускал sing-box, вернёт Ok сразу.
pub async fn singbox_stop() -> Result<()> {
    let resp = send(HelperRequest::SingBoxStop).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// Cleanup orphan TUN-ресурсов: адаптеры с префиксом `nemefisto-` и
/// half-default routes через `198.18.0.1`. Часть UI-кнопки
/// «восстановить сеть». Безопасно вызывать только когда VPN не активен
/// (иначе порвёт активный туннель).
pub async fn orphan_cleanup() -> Result<()> {
    let resp = send(HelperRequest::OrphanCleanup).await?;
    match resp {
        HelperResponse::Ok => Ok(()),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

/// 14.E: проверка остатков WFP-фильтров от прошлой сессии. Best-effort,
/// без побочных эффектов. Возвращает `Ok(true)` если sublayer с нашим
/// GUID существует в persistent WFP store. Используется для UI-сигнала
/// в crash-recovery диалоге.
pub async fn wfp_query_orphan() -> Result<bool> {
    let resp = send(HelperRequest::WfpQueryOrphan).await?;
    match resp {
        HelperResponse::WfpOrphan { has_orphan } => Ok(has_orphan),
        HelperResponse::Error { message } => bail!("{message}"),
        other => bail!("неожиданный ответ helper: {other:?}"),
    }
}

