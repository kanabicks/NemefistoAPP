//! Tauri commands, доступные из фронтенда через `invoke`.

use serde::Serialize;
use tauri::State;

use crate::config::subscription::fetch_and_parse;
use crate::config::xray_config;
use crate::config::{HwidState, ProxyEntry, SubscriptionState};
use crate::platform;
use crate::vpn::{find_free_port, XrayState};

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
/// сгенерированного HWID. Нужен чтобы сервер подписки распознал устройство
/// (Happ-совместимые сервисы держат whitelist HWID-ов).
#[tauri::command]
pub async fn fetch_subscription(
    url: String,
    hwid_override: Option<String>,
    hwid: State<'_, HwidState>,
    sub: State<'_, SubscriptionState>,
) -> Result<Vec<ProxyEntry>, String> {
    let effective_hwid = hwid_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&hwid.0);

    let servers = fetch_and_parse(&url, effective_hwid)
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
///
/// Автоматически находит свободные порты начиная с 1080/1087.
#[tauri::command]
pub fn connect(
    server_index: usize,
    mode: String,
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

    // xray-json: патчим внешний конфиг (порты, убираем geoip/geosite/observatory)
    // иначе: генерируем конфиг из ProxyEntry
    let (config_json, socks_port, http_port) = if entry.protocol == "xray-json" {
        let patched = xray_config::patch_xray_json(entry.raw.clone(), default_socks, default_http);
        (patched, default_socks, default_http)
    } else {
        let cfg = xray_config::build(&entry, default_socks, default_http)
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
