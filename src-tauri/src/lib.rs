//! Точка входа Tauri-приложения.
//!
//! Здесь подключаются плагины (shell, opener), регистрируется application state
//! и Tauri commands. Бизнес-логика живёт в модулях `vpn`, `config`, `platform`.

mod config;
mod ipc;
mod vpn;

use tauri::Manager;

use config::hwid::load_or_create;
use config::{HwidState, SubscriptionState};
use ipc::commands::{fetch_subscription, get_servers, is_xray_running, start_xray, stop_xray};
use vpn::XrayState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(XrayState::new())
        .manage(SubscriptionState::new())
        .setup(|app| {
            let hwid = load_or_create().unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
            app.manage(HwidState(hwid));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_xray,
            stop_xray,
            is_xray_running,
            fetch_subscription,
            get_servers,
        ])
        .run(tauri::generate_context!())
        .expect("ошибка инициализации Tauri runtime")
}
