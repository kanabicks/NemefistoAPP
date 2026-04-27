//! Tauri commands, доступные из фронтенда через `invoke`.

use tauri::State;

use crate::config::subscription::fetch_and_parse;
use crate::config::{HwidState, ProxyEntry, SubscriptionState};
use crate::vpn::XrayState;

/// Запустить Xray sidecar с минимальным конфигом.
#[tauri::command]
pub fn start_xray(app: tauri::AppHandle, state: State<'_, XrayState>) -> Result<(), String> {
    state.start(&app)
}

/// Остановить Xray sidecar.
#[tauri::command]
pub fn stop_xray(state: State<'_, XrayState>) -> Result<(), String> {
    state.stop()
}

/// Запущен ли Xray прямо сейчас.
#[tauri::command]
pub fn is_xray_running(state: State<'_, XrayState>) -> bool {
    state.is_running()
}

/// Скачать подписку по URL, распарсить и сохранить список серверов.
///
/// Возвращает актуальный список серверов фронтенду.
#[tauri::command]
pub async fn fetch_subscription(
    url: String,
    hwid: State<'_, HwidState>,
    sub: State<'_, SubscriptionState>,
) -> Result<Vec<ProxyEntry>, String> {
    let servers = fetch_and_parse(&url, &hwid.0)
        .await
        .map_err(|e| e.to_string())?;

    *sub.servers.lock().map_err(|e| e.to_string())? = servers.clone();
    Ok(servers)
}

/// Вернуть закешированный список серверов без обращения к серверу.
#[tauri::command]
pub fn get_servers(sub: State<'_, SubscriptionState>) -> Vec<ProxyEntry> {
    sub.servers
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}
