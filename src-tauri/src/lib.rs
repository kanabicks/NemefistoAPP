//! Точка входа Tauri-приложения.

mod config;
mod ipc;
mod platform;
mod vpn;

use tauri::Manager;

use config::hwid::load_or_create;
use config::{HwidState, SubscriptionState};
use ipc::commands::{
    connect, disconnect, fetch_subscription, get_hwid, get_servers, is_xray_running,
    ping_servers, read_xray_log,
};
use vpn::XrayState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(XrayState::new())
        .manage(SubscriptionState::new())
        .setup(|app| {
            let hwid = load_or_create().unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
            app.manage(HwidState(hwid));

            // В dev-режиме регистрируем nemefisto:// в HKCU\Software\Classes
            // для текущего пользователя. Production-инсталлятор пишет
            // регистрацию сам через bundle-metadata.
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register("nemefisto");
            }
            Ok(())
        })
        // Очищаем системный прокси и убиваем Xray при закрытии окна
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                let xray = window.state::<XrayState>();
                let _ = xray.stop();
                let _ = platform::proxy::clear_system_proxy();
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            is_xray_running,
            fetch_subscription,
            get_servers,
            get_hwid,
            ping_servers,
            read_xray_log,
        ])
        .run(tauri::generate_context!())
        .expect("ошибка инициализации Tauri runtime")
}
