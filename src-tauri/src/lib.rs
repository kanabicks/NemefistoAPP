//! Точка входа Tauri-приложения.

mod config;
mod ipc;
mod platform;
mod vpn;

use tauri::Manager;

use config::hwid::load_or_create;
use config::{HwidState, SubscriptionState};
use ipc::commands::{
    autostart_disable, autostart_enable, autostart_is_enabled, connect, discard_proxy_backup,
    disconnect, fetch_subscription, get_hwid, get_servers, get_subscription_meta,
    has_proxy_backup, is_xray_running, ping_servers, read_xray_log, restore_proxy_backup,
    secure_storage_delete, secure_storage_get, secure_storage_set, tray_set_status,
};
use vpn::{MihomoState, XrayState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(XrayState::new())
        .manage(MihomoState::new())
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
            // 6.C: запускаем watcher смены сети. Polling default-route
            // каждые 5 сек; при смене интерфейса emit-ится событие
            // `network-changed` во фронт, который при активном VPN
            // делает reconnect.
            platform::network_watcher::start(app.handle().clone());
            // 13.A: системный трей. Создаём один раз; меню обновляется
            // через `tray_set_status` команду из фронта при смене VPN-статуса.
            platform::tray::init(app.handle())?;
            Ok(())
        })
        // 13.A: закрытие окна → сворачиваем в трей, не выходим из приложения.
        // Outright выход возможен только через пункт «Выйти» в меню трея —
        // там же делается полный shutdown (Xray/Mihomo/proxy/exit).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            is_xray_running,
            fetch_subscription,
            get_servers,
            get_subscription_meta,
            get_hwid,
            ping_servers,
            read_xray_log,
            has_proxy_backup,
            restore_proxy_backup,
            discard_proxy_backup,
            secure_storage_get,
            secure_storage_set,
            secure_storage_delete,
            autostart_is_enabled,
            autostart_enable,
            autostart_disable,
            tray_set_status,
        ])
        .run(tauri::generate_context!())
        .expect("ошибка инициализации Tauri runtime")
}
