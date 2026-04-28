//! Tauri commands, доступные из фронтенда через `invoke`.

use serde::Serialize;
use tauri::State;

use crate::config::subscription::fetch_and_parse;
use crate::config::xray_config;
use crate::config::{HwidState, ProxyEntry, SubscriptionState};
use crate::platform;
use crate::vpn::{find_free_port, ping_entry, XrayState};

// ─── Результаты команд ────────────────────────────────────────────────────────

/// Возвращается фронтенду после успешного подключения.
#[derive(Serialize)]
pub struct ConnectResult {
    pub socks_port: u16,
    pub http_port: u16,
    pub server_name: String,
}

// ─── Подписка ─────────────────────────────────────────────────────────────────

/// Скачать подписку по URL, распарсить и сохранить список серверов.
///
/// `hwid_override` — если задан и непустой, используется вместо локально
/// сгенерированного MachineGuid (нужен только для разработки / переноса
/// с другого клиента).
/// `user_agent` — позволяет переопределить дефолт `Happ/2.7.0`.
/// `send_hwid` — если false, заголовок `x-hwid` не отправляется.
#[tauri::command]
pub async fn fetch_subscription(
    url: String,
    hwid_override: Option<String>,
    user_agent: Option<String>,
    send_hwid: Option<bool>,
    hwid: State<'_, HwidState>,
    sub: State<'_, SubscriptionState>,
) -> Result<Vec<ProxyEntry>, String> {
    let effective_hwid = hwid_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&hwid.0);

    let ua = user_agent.unwrap_or_default();
    let send = send_hwid.unwrap_or(true);

    let servers = fetch_and_parse(&url, effective_hwid, &ua, send)
        .await
        .map_err(|e| e.to_string())?;

    *sub.servers.lock().map_err(|e| e.to_string())? = servers.clone();
    Ok(servers)
}

/// Вернуть закешированный список серверов без сетевого запроса.
#[tauri::command]
pub fn get_servers(sub: State<'_, SubscriptionState>) -> Vec<ProxyEntry> {
    sub.servers.lock().map(|g| g.clone()).unwrap_or_default()
}

// ─── Подключение ──────────────────────────────────────────────────────────────

/// Подключиться к серверу с указанным индексом в режиме `mode`.
///
/// `mode` = "proxy" — системный SOCKS5 + HTTP прокси через реестр.
/// `mode` = "tun"   — TUN-режим (Этап 4, пока не реализован).
/// `allow_lan` — если `Some(true)`, inbound слушает 0.0.0.0 вместо 127.0.0.1
/// (даёт другим устройствам в локальной сети использовать наш прокси).
///
/// Автоматически находит свободные порты начиная с 1080/1087.
#[tauri::command]
pub fn connect(
    server_index: usize,
    mode: String,
    allow_lan: Option<bool>,
    app: tauri::AppHandle,
    xray: State<'_, XrayState>,
    sub: State<'_, SubscriptionState>,
) -> Result<ConnectResult, String> {
    // Клонируем ProxyEntry, чтобы сразу освободить lock на список серверов
    let entry = {
        let servers = sub.servers.lock().map_err(|e| e.to_string())?;
        servers
            .get(server_index)
            .cloned()
            .ok_or_else(|| format!("сервер #{server_index} не найден в списке"))?
    };

    let default_socks = find_free_port(1080);
    let default_http = find_free_port(1087);
    let listen = if allow_lan.unwrap_or(false) { "0.0.0.0" } else { "127.0.0.1" };

    // xray-json: патчим внешний конфиг (порты, убираем geoip/geosite/observatory)
    // иначе: генерируем конфиг из ProxyEntry
    let (config_json, socks_port, http_port) = if entry.protocol == "xray-json" {
        let patched = xray_config::patch_xray_json(entry.raw.clone(), default_socks, default_http, listen);
        (patched, default_socks, default_http)
    } else {
        let cfg = xray_config::build(&entry, default_socks, default_http, listen)
            .map_err(|e| e.to_string())?;
        (cfg.json, cfg.socks_port, cfg.http_port)
    };

    xray.start_with_config(&app, &config_json, socks_port, http_port)?;

    match mode.as_str() {
        "proxy" => {
            platform::proxy::set_system_proxy(socks_port, http_port)
                .map_err(|e| e.to_string())?;
        }
        "tun" => {
            return Err("TUN-режим будет реализован на Этапе 4".to_string());
        }
        other => {
            return Err(format!("неизвестный режим: {other}"));
        }
    }

    Ok(ConnectResult {
        socks_port,
        http_port,
        server_name: entry.name,
    })
}

/// Отключиться: остановить Xray и сбросить системный прокси.
#[tauri::command]
pub fn disconnect(xray: State<'_, XrayState>) -> Result<(), String> {
    xray.stop()?;
    platform::proxy::clear_system_proxy().map_err(|e| e.to_string())?;
    Ok(())
}

/// Запущен ли Xray прямо сейчас.
#[tauri::command]
pub fn is_xray_running(xray: State<'_, XrayState>) -> bool {
    xray.is_running()
}

/// Вернуть HWID устройства (Windows MachineGuid либо локально сохранённый UUID).
/// Используется UI для отображения и копирования.
#[tauri::command]
pub fn get_hwid(hwid: State<'_, HwidState>) -> String {
    hwid.0.clone()
}

/// Прочитать последние N байт лога Xray (`%TEMP%\NemefistoVPN\xray-stderr.log`).
///
/// Возвращает строку из последних 32 КБ файла. Если файл не существует —
/// пустую строку. Используется UI для отображения логов.
#[tauri::command]
pub fn read_xray_log() -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom};

    let path = std::env::temp_dir()
        .join("NemefistoVPN")
        .join("xray-stderr.log");

    if !path.exists() {
        return Ok(String::new());
    }

    let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let max = 32 * 1024;
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Пинговать все серверы из текущей подписки параллельно (TCP-connect).
///
/// Возвращает массив той же длины и порядка что `get_servers`. Для каждого
/// сервера: время отклика в мс или `None`, если адрес не извлекается /
/// сервер не ответил за 2.5 секунды.
#[tauri::command]
pub async fn ping_servers(
    sub: State<'_, SubscriptionState>,
) -> Result<Vec<Option<u32>>, String> {
    let entries: Vec<ProxyEntry> = {
        let g = sub.servers.lock().map_err(|e| e.to_string())?;
        g.clone()
    };

    let futures = entries.iter().map(ping_entry);
    let results = futures::future::join_all(futures).await;
    Ok(results)
}
