//! Управление процессом Xray-core (sidecar).
//!
//! На Этапе 1 Xray запускается и останавливается по кнопке в UI с минимальным
//! статическим конфигом (SOCKS5 на 127.0.0.1:1080, outbound — freedom). Реальный
//! VPN-outbound подключим на Этапе 3, а долгоживущий sidecar (принцип №1 из
//! CLAUDE.md) — на Этапе 5.

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Глобальный state Xray sidecar внутри Tauri AppState.
pub struct XrayState {
    child: Mutex<Option<CommandChild>>,
}

impl XrayState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }

    /// Запустить Xray. Если уже запущен — no-op.
    pub fn start(&self, app: &AppHandle) -> Result<(), String> {
        let mut guard = self
            .child
            .lock()
            .map_err(|e| format!("XrayState mutex отравлен: {e}"))?;
        if guard.is_some() {
            return Ok(());
        }

        // В dev-режиме Tauri не копирует resources/ в target/, читаем напрямую
        // из исходников. В release Tauri бандлит файлы и resource_dir() указывает
        // на правильное место внутри установленного приложения.
        #[cfg(debug_assertions)]
        let config_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("xray-config.json");
        #[cfg(not(debug_assertions))]
        let config_path = app
            .path()
            .resource_dir()
            .map_err(|e| format!("не удалось получить resource_dir: {e}"))?
            .join("xray-config.json");

        let config_str = config_path
            .to_str()
            .ok_or_else(|| "путь к xray-config.json содержит не-UTF-8 символы".to_string())?;

        let (mut rx, child) = app
            .shell()
            .sidecar("xray")
            .map_err(|e| format!("sidecar xray не зарегистрирован: {e}"))?
            .args(["-config", config_str])
            .spawn()
            .map_err(|e| format!("не удалось запустить xray: {e}"))?;

        *guard = Some(child);
        drop(guard);

        // Логи Xray на Этапе 1 — в stderr приложения. На Этапе 5+ заведём
        // отдельный канал через crate `tracing`.
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        eprintln!("[xray:stdout] {}", String::from_utf8_lossy(&line));
                    }
                    CommandEvent::Stderr(line) => {
                        eprintln!("[xray:stderr] {}", String::from_utf8_lossy(&line));
                    }
                    CommandEvent::Terminated(payload) => {
                        eprintln!(
                            "[xray] terminated: code={:?}, signal={:?}",
                            payload.code, payload.signal
                        );
                        let state = app_handle.state::<XrayState>();
                        if let Ok(mut guard) = state.child.lock() {
                            *guard = None;
                        }
                        break;
                    }
                    CommandEvent::Error(err) => {
                        eprintln!("[xray:error] {err}");
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    /// Остановить Xray. Если не запущен — no-op.
    pub fn stop(&self) -> Result<(), String> {
        let mut guard = self
            .child
            .lock()
            .map_err(|e| format!("XrayState mutex отравлен: {e}"))?;
        if let Some(child) = guard.take() {
            child.kill().map_err(|e| format!("kill xray: {e}"))?;
        }
        Ok(())
    }

    /// Запущен ли Xray прямо сейчас (по нашему учёту).
    pub fn is_running(&self) -> bool {
        match self.child.lock() {
            Ok(guard) => guard.is_some(),
            Err(_) => false,
        }
    }
}
