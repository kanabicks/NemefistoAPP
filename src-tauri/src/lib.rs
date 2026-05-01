//! Точка входа Tauri-приложения.

mod config;
mod ipc;
mod platform;
mod vpn;

use tauri::{Emitter, Manager};

use config::hwid::load_or_create;
use config::{HwidState, SubscriptionState};
use ipc::commands::{
    autostart_disable, autostart_enable, autostart_is_enabled, check_routing_conflicts, connect,
    detect_competing_vpns, discard_proxy_backup, disconnect, export_diagnostics,
    export_settings_to_documents, fetch_settings_backup, fetch_subscription, geofiles_refresh,
    geofiles_status, get_hwid, get_recovery_state, get_servers, get_subscription_meta,
    has_proxy_backup, hide_floating_window, is_xray_running, kill_switch_apply,
    kill_switch_force_cleanup, kill_switch_heartbeat, leak_test, ping_servers, read_xray_log,
    recover_network, restore_proxy_backup, routing_add_static, routing_add_url, routing_list,
    routing_refresh, routing_remove, routing_set_active, secure_storage_delete, secure_storage_get,
    secure_storage_set, show_floating_window, tray_set_status, KillSwitchState,
};
use vpn::{MihomoState, XrayState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        // 13.N: глобальные горячие клавиши (Ctrl+Shift+V toggle VPN и др.).
        // Регистрация конкретных комбинаций — из фронта через
        // `@tauri-apps/plugin-global-shortcut`, при изменении настроек
        // (см. lib/hooks/useGlobalShortcuts.ts).
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(XrayState::new())
        .manage(MihomoState::new())
        .manage(SubscriptionState::new())
        .manage(KillSwitchState::new())
        .manage(config::routing_store::RoutingStoreState::new())
        .setup(|app| {
            // Self-healing на старте: сначала захватываем lockfile, чтобы
            // понять упала ли прошлая сессия. Если да — синхронно (до
            // показа UI) чиним отравленный системный прокси. WFP-фильтры
            // и orphan TUN-ресурсы помощник чистит сам в `service.rs`
            // при старте сервиса (см. firewall::cleanup_on_startup и
            // tun::cleanup_orphan_resources).
            let acquire_outcome = platform::session_lock::acquire();
            if matches!(
                acquire_outcome,
                platform::session_lock::AcquireOutcome::PreviousSessionCrashed
            ) {
                // Если регистрация системного прокси указывает на наш
                // диапазон портов — это однозначно наш orphan от
                // упавшего connect'а, чистим без лишних вопросов.
                if platform::proxy::is_proxy_pointing_to_us() {
                    if let Err(e) = platform::proxy::force_clear_system_proxy() {
                        eprintln!("[startup self-healing] force clear proxy: {e:#}");
                    } else {
                        eprintln!("[startup self-healing] orphan системный прокси очищен");
                    }
                }
                // Backup от прошлой сессии не трогаем: фронт сам покажет
                // диалог CrashRecoveryDialog когда подключится к команде
                // `has_proxy_backup` — пользователь решит restore/discard.
            }

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
            // 13.O: измеритель скорости. Эмитит `bandwidth-tick` каждую
            // секунду — для floating-окна и опционально main-окна.
            platform::bandwidth::start(app.handle().clone());

            // 11.C: scheduler авто-обновления routing-профилей и geofiles.
            // Использует Notify wake-up для немедленной реакции на add/refresh.
            // Сохранённый shutdown-sender утечёт — scheduler-task остановится
            // при exit вместе с tokio runtime'ом.
            {
                let store_state =
                    app.state::<config::routing_store::RoutingStoreState>();
                let _shutdown = config::routing_store::spawn_scheduler(
                    store_state.inner.clone(),
                    store_state.wake.clone(),
                );
            }
            // 13.O: создаём floating-окно один раз, скрытым. Toggle
            // через команду `show_floating_window`/`hide_floating_window`
            // (из Settings → appearance).
            let _ = tauri::WebviewWindowBuilder::new(
                app,
                "floating",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Nemefisto")
            .inner_size(190.0, 52.0)
            .min_inner_size(170.0, 48.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .visible(false)
            .build()?;
            Ok(())
        })
        // 13.A: закрытие главного окна → сворачиваем в трей, не выходим
        // из приложения. Outright выход возможен только через пункт
        // «Выйти» в меню трея — там же делается полный shutdown.
        // 13.O: закрытие floating-окна (×) → скрываем и эмитим
        // `floating-closed` чтобы фронт сбросил `settings.floatingWindow`.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                if window.label() == "floating" {
                    let _ = window.app_handle().emit("floating-closed", ());
                }
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
            show_floating_window,
            hide_floating_window,
            leak_test,
            kill_switch_force_cleanup,
            kill_switch_heartbeat,
            kill_switch_apply,
            recover_network,
            get_recovery_state,
            export_diagnostics,
            export_settings_to_documents,
            fetch_settings_backup,
            detect_competing_vpns,
            check_routing_conflicts,
            routing_list,
            routing_add_static,
            routing_add_url,
            routing_remove,
            routing_set_active,
            routing_refresh,
            geofiles_refresh,
            geofiles_status,
        ])
        .build(tauri::generate_context!())
        .expect("ошибка инициализации Tauri runtime")
        .run(|_handle, event| {
            // Освобождаем lockfile при clean exit. При kill -9 / hard crash
            // hook не вызовется — следующий старт сам обнаружит stale lock
            // и запустит self-healing.
            if let tauri::RunEvent::Exit = event {
                platform::session_lock::release();
            }
        });
}
