//! Управление процессом Mihomo (Clash Meta) sidecar — этап 8.B.
//!
//! Симметричен `xray.rs`: принимает готовый YAML-конфиг, пишет в файл
//! `%TEMP%\NemefistoVPN\mihomo-config.yaml` и запускает sidecar
//! `mihomo` (binary `mihomo-x86_64-pc-windows-msvc.exe`). Логи stderr —
//! в `%TEMP%\NemefistoVPN\mihomo-stderr.log`.
//!
//! Один движок на сессию (Xray ИЛИ Mihomo), что выбран — определяется в
//! `commands.rs::connect()`. Mihomo используется когда сервер из подписки
//! имеет `engine_compat = ["mihomo"]` (TUIC / AnyTLS / Mieru) либо когда
//! пользователь явно выбрал Mihomo через Settings.
//!
//! У Mihomo один объединённый порт `mixed-port` для SOCKS5+HTTP, поэтому
//! `MihomoState` хранит только один порт (в отличие от `XrayState` с двумя).

use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Глобальный state Mihomo sidecar.
///
/// `current_pid` нужен для защиты от race: при быстром перезапуске старый
/// listener Terminated не должен затереть state нового процесса.
///
/// `helper_spawned` — 13.L флаг для built-in TUN-режима. Когда `true`,
/// mihomo запущен helper-сервисом (SYSTEM), не через Tauri sidecar.
/// `child` в этом случае пуст (Tauri процессом не владеет), но
/// `is_running()` всё равно возвращает true — UI/connect-checks
/// должны видеть состояние корректно.
pub struct MihomoState {
    child: Mutex<Option<CommandChild>>,
    current_pid: Mutex<Option<u32>>,
    /// `mixed-port` Mihomo: один сокет на SOCKS5 и HTTP одновременно
    /// (стандартная фича clash-style ядра, не требует двух inbound'ов).
    pub mixed_port: Mutex<u16>,
    /// 13.L: mihomo запущен helper'ом (built-in TUN). Set/cleared в
    /// connect/disconnect, влияет только на `is_running()`.
    helper_spawned: AtomicBool,
}

impl MihomoState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            current_pid: Mutex::new(None),
            mixed_port: Mutex::new(7890),
            helper_spawned: AtomicBool::new(false),
        }
    }

    /// 13.L: пометить что mihomo сейчас запущен helper-сервисом
    /// (built-in TUN-режим). На Tauri-стороне Child нет, но `is_running`
    /// должен возвращать true.
    pub fn mark_helper_spawned(&self, on: bool) {
        self.helper_spawned.store(on, Ordering::SeqCst);
    }

    /// Запустить Mihomo с указанным YAML-конфигом.
    ///
    /// Если уже запущен — останавливает перед перезапуском. Конфиг
    /// сохраняется в `%TEMP%\NemefistoVPN\mihomo-config.yaml`.
    pub fn start_with_config(
        &self,
        app: &AppHandle,
        config_yaml: &str,
        mixed_port: u16,
    ) -> Result<(), String> {
        self.stop()?;

        let tmp_dir = std::env::temp_dir().join("NemefistoVPN");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("не удалось создать %TEMP%\\NemefistoVPN: {e}"))?;
        let config_path = tmp_dir.join("mihomo-config.yaml");
        std::fs::write(&config_path, config_yaml)
            .map_err(|e| format!("запись mihomo-конфига: {e}"))?;

        let config_path_str = config_path
            .to_str()
            .ok_or_else(|| "путь к mihomo-конфигу содержит не-UTF-8 символы".to_string())?;

        // Mihomo требует «директорию данных» где хранятся geoip/geosite и cache.db.
        // Используем тот же %TEMP%\NemefistoVPN — Mihomo сам создаст при необходимости.
        let data_dir_str = tmp_dir
            .to_str()
            .ok_or_else(|| "путь к data-dir содержит не-UTF-8 символы".to_string())?;

        let stderr_log_path = tmp_dir.join("mihomo-stderr.log");
        let stderr_log: Arc<Mutex<File>> = Arc::new(Mutex::new(
            File::create(&stderr_log_path)
                .map_err(|e| format!("создание mihomo stderr-лога: {e}"))?,
        ));

        let (mut rx, child) = app
            .shell()
            .sidecar("mihomo")
            .map_err(|e| format!("sidecar mihomo не зарегистрирован: {e}"))?
            .args(["-f", config_path_str, "-d", data_dir_str])
            .spawn()
            .map_err(|e| format!("не удалось запустить mihomo: {e}"))?;

        let my_pid = child.pid();
        eprintln!("[mihomo] запущен pid={my_pid}, stderr-лог: {stderr_log_path:?}");

        {
            let mut g = self.child.lock().map_err(|e| format!("mutex: {e}"))?;
            *g = Some(child);
        }
        *self
            .current_pid
            .lock()
            .map_err(|e| format!("mutex: {e}"))? = Some(my_pid);
        *self.mixed_port.lock().map_err(|e| format!("mutex: {e}"))? = mixed_port;

        let app_handle = app.clone();
        let stderr_log_clone = stderr_log.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    // Mihomo (Go logrus) пишет ВСЁ в stdout — info, warning,
                    // error. Чтобы пользователь мог открыть mihomo-stderr.log
                    // и увидеть «provider download failed / config parse
                    // error», складываем оба stream'а в один файл с
                    // префиксом. Без этого `out`-канал терялся в eprintln.
                    CommandEvent::Stdout(line) => {
                        let s = String::from_utf8_lossy(&line);
                        eprintln!("[mihomo:out] {s}");
                        if let Ok(mut f) = stderr_log_clone.lock() {
                            let _ = writeln!(f, "{s}");
                            let _ = f.flush();
                        }
                    }
                    CommandEvent::Stderr(line) => {
                        let s = String::from_utf8_lossy(&line);
                        eprintln!("[mihomo:err] {s}");
                        if let Ok(mut f) = stderr_log_clone.lock() {
                            let _ = writeln!(f, "{s}");
                            let _ = f.flush();
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        eprintln!(
                            "[mihomo] завершён pid={my_pid}: code={:?}, signal={:?}",
                            payload.code, payload.signal
                        );
                        if let Ok(mut f) = stderr_log_clone.lock() {
                            let _ = writeln!(
                                f,
                                "--- terminated code={:?} signal={:?} ---",
                                payload.code, payload.signal
                            );
                            let _ = f.flush();
                        }
                        let state = app_handle.state::<MihomoState>();
                        let is_current = state
                            .current_pid
                            .lock()
                            .map(|g| *g == Some(my_pid))
                            .unwrap_or(false);
                        if is_current {
                            if let Ok(mut g) = state.child.lock() {
                                *g = None;
                            }
                            if let Ok(mut g) = state.current_pid.lock() {
                                *g = None;
                            }
                            // Сбрасываем system proxy только если упал актуальный процесс
                            // (см. xray.rs — та же защита от race при быстром реконнекте).
                            let _ = crate::platform::proxy::clear_system_proxy();
                        } else {
                            eprintln!(
                                "[mihomo] pid={my_pid} устаревший — не трогаем state нового процесса"
                            );
                        }
                        break;
                    }
                    CommandEvent::Error(err) => {
                        eprintln!("[mihomo:error] {err}");
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    /// Остановить Mihomo. Если не запущен — no-op.
    pub fn stop(&self) -> Result<(), String> {
        let mut g = self.child.lock().map_err(|e| format!("mutex: {e}"))?;
        if let Some(child) = g.take() {
            let pid = child.pid();
            eprintln!("[mihomo] kill pid={pid} (явный stop)");
            child.kill().map_err(|e| format!("kill mihomo: {e}"))?;
        }
        if let Ok(mut g) = self.current_pid.lock() {
            *g = None;
        }
        Ok(())
    }

    /// Запущен ли Mihomo прямо сейчас (как Tauri-sidecar ИЛИ через helper).
    pub fn is_running(&self) -> bool {
        if self.helper_spawned.load(Ordering::SeqCst) {
            return true;
        }
        self.child.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}
