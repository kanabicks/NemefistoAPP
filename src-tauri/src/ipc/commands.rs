//! Tauri commands, доступные из фронтенда через `invoke`.
//!
//! На Этапе 1 — только управление Xray sidecar.

use tauri::{AppHandle, State};

use crate::vpn::XrayState;

/// Запустить Xray sidecar с минимальным конфигом.
#[tauri::command]
pub fn start_xray(app: AppHandle, state: State<'_, XrayState>) -> Result<(), String> {
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
