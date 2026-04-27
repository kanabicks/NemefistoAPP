//! Точка входа Tauri-приложения.
//!
//! Здесь подключаются плагины (shell, opener), регистрируется application state
//! и Tauri commands. Бизнес-логика живёт в модулях `vpn`, `config`, `platform`.

mod ipc;
mod vpn;

use ipc::commands::{is_xray_running, start_xray, stop_xray};
use vpn::XrayState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(XrayState::new())
        .invoke_handler(tauri::generate_handler![
            start_xray,
            stop_xray,
            is_xray_running,
        ])
        .run(tauri::generate_context!())
        // Паника здесь = невозможность инициализировать Tauri-runtime,
        // продолжать работу нет смысла.
        .expect("ошибка инициализации Tauri runtime");
}
