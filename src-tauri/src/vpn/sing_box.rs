//! Управление процессом sing-box sidecar.
//!
//! Архитектурно идентичен `mihomo.rs`: принимает готовый JSON-конфиг,
//! пишет в файл `%TEMP%\NemefistoVPN\sing-box-config.json` и запускает
//! sidecar `sing-box` (binary `sing-box-x86_64-pc-windows-msvc.exe`).
//! Логи stdout+stderr — в `%TEMP%\NemefistoVPN\sing-box-stderr.log`.
//!
//! Один движок на сессию (sing-box ИЛИ Mihomo), что выбран — определяется
//! в `commands.rs::connect()`. sing-box используется когда сервер из
//! подписки имеет совместимость с sing-box (большинство современных
//! протоколов — vless+reality, hy2, tuic, wg и т.д.) и пользователь не
//! выбрал явно Mihomo.
//!
//! У sing-box один объединённый порт `mixed`-inbound для SOCKS5+HTTP
//! (как и у Mihomo), поэтому `SingBoxState` хранит только один порт
//! (в отличие от XrayState с двумя).
//!
//! `helper_spawned` — для built-in TUN-режима. Когда `true`, sing-box
//! запущен helper-сервисом (SYSTEM), не через Tauri sidecar. Tauri
//! процессом не владеет, но `is_running()` всё равно возвращает true —
//! UI/connect-checks должны видеть состояние корректно.

use std::fs::File;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Является ли строка лога безобидным network-noise который не стоит
/// показывать пользователю в консоли. Эти ошибки означают что клиент
/// (браузер/приложение) сам разорвал TCP-соединение до того как
/// sing-box успел отдать ответ — типичный prefetch-сценарий, не баг.
fn is_network_noise(line: &str) -> bool {
    // wsasend / WSAECONNABORTED 10053
    line.contains("wsasend: An established connection was aborted")
        || line.contains("forcibly closed by the remote host")
        || line.contains("connection upload closed")
        || line.contains("connection download closed")
        // EOF на upload/download — клиент закрыл нормально, sing-box логирует
        || (line.contains("connection: ") && line.contains("EOF"))
}

/// Глобальный state sing-box sidecar.
pub struct SingBoxState {
    child: Mutex<Option<CommandChild>>,
    current_pid: Mutex<Option<u32>>,
    /// `mixed`-inbound: один сокет на SOCKS5 и HTTP одновременно.
    pub mixed_port: Mutex<u16>,
    /// sing-box запущен helper-сервисом (built-in TUN-режим). Set/cleared
    /// в connect/disconnect, влияет только на `is_running()`.
    helper_spawned: AtomicBool,
}

impl SingBoxState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            current_pid: Mutex::new(None),
            mixed_port: Mutex::new(0),
            helper_spawned: AtomicBool::new(false),
        }
    }

    /// Пометить что sing-box сейчас запущен helper-сервисом
    /// (built-in TUN-режим). На Tauri-стороне Child нет, но `is_running`
    /// должен возвращать true.
    pub fn mark_helper_spawned(&self, on: bool) {
        self.helper_spawned.store(on, Ordering::SeqCst);
    }

    /// Запустить sing-box с указанным JSON-конфигом.
    ///
    /// Если уже запущен — останавливает перед перезапуском. Конфиг
    /// сохраняется в `%TEMP%\NemefistoVPN\sing-box-config.json`.
    pub fn start_with_config(
        &self,
        app: &AppHandle,
        config_json: &str,
        mixed_port: u16,
    ) -> Result<(), String> {
        self.stop()?;

        let tmp_dir = std::env::temp_dir().join("NemefistoVPN");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("не удалось создать %TEMP%\\NemefistoVPN: {e}"))?;
        let config_path = tmp_dir.join("sing-box-config.json");
        std::fs::write(&config_path, config_json)
            .map_err(|e| format!("запись sing-box-конфига: {e}"))?;

        let config_path_str = config_path
            .to_str()
            .ok_or_else(|| "путь к sing-box-конфигу содержит не-UTF-8 символы".to_string())?;

        // sing-box `-D` рабочая директория — там кешируются скачанные
        // rule-set'ы (geosite-ru.srs и т.д.) между запусками.
        let data_dir_str = tmp_dir
            .to_str()
            .ok_or_else(|| "путь к data-dir содержит не-UTF-8 символы".to_string())?;

        let stderr_log_path = tmp_dir.join("sing-box-stderr.log");
        let stderr_log: Arc<Mutex<File>> = Arc::new(Mutex::new(
            File::create(&stderr_log_path)
                .map_err(|e| format!("создание sing-box stderr-лога: {e}"))?,
        ));

        let (mut rx, child) = app
            .shell()
            .sidecar("sing-box")
            .map_err(|e| format!("sidecar sing-box не зарегистрирован: {e}"))?
            .args(["run", "-c", config_path_str, "-D", data_dir_str])
            .spawn()
            .map_err(|e| format!("не удалось запустить sing-box: {e}"))?;

        let my_pid = child.pid();
        eprintln!("[sing-box] запущен pid={my_pid}, stderr-лог: {stderr_log_path:?}");

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
                    // sing-box (Go logrus) пишет ВСЁ в stderr — info,
                    // warning, error. Stdout обычно пустой. Складываем
                    // оба stream'а в один файл для диагностики.
                    //
                    // В консоль (eprintln) фильтруем безобидный
                    // network-noise — эти ошибки означают что клиент
                    // (браузер) сам закрыл соединение до того как
                    // sing-box успел ответить. Это норма для prefetch'а /
                    // переключения вкладок, не наша ошибка. В файл-лог
                    // пишем всегда — на случай глубокой диагностики
                    // (`%TEMP%\NemefistoVPN\sing-box-stderr.log`).
                    CommandEvent::Stdout(line) => {
                        let s = String::from_utf8_lossy(&line);
                        if !is_network_noise(&s) {
                            eprintln!("[sing-box:out] {s}");
                        }
                        if let Ok(mut f) = stderr_log_clone.lock() {
                            let _ = writeln!(f, "{s}");
                            let _ = f.flush();
                        }
                    }
                    CommandEvent::Stderr(line) => {
                        let s = String::from_utf8_lossy(&line);
                        if !is_network_noise(&s) {
                            eprintln!("[sing-box:err] {s}");
                        }
                        if let Ok(mut f) = stderr_log_clone.lock() {
                            let _ = writeln!(f, "{s}");
                            let _ = f.flush();
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        eprintln!(
                            "[sing-box] завершён pid={my_pid}: code={:?}, signal={:?}",
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
                        let state = app_handle.state::<SingBoxState>();
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
                            // Сбрасываем system proxy только если упал
                            // актуальный процесс (защита от race при
                            // быстром реконнекте).
                            let _ = crate::platform::proxy::clear_system_proxy();
                        } else {
                            eprintln!(
                                "[sing-box] pid={my_pid} устаревший — не трогаем state нового процесса"
                            );
                        }
                        break;
                    }
                    CommandEvent::Error(err) => {
                        eprintln!("[sing-box:error] {err}");
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    /// Остановить sing-box. Если не запущен — no-op.
    pub fn stop(&self) -> Result<(), String> {
        let mut g = self.child.lock().map_err(|e| format!("mutex: {e}"))?;
        if let Some(child) = g.take() {
            let pid = child.pid();
            eprintln!("[sing-box] kill pid={pid} (явный stop)");
            child.kill().map_err(|e| format!("kill sing-box: {e}"))?;
        }
        if let Ok(mut g) = self.current_pid.lock() {
            *g = None;
        }
        Ok(())
    }

    /// Запущен ли sing-box прямо сейчас (как Tauri-sidecar ИЛИ через helper).
    pub fn is_running(&self) -> bool {
        if self.helper_spawned.load(Ordering::SeqCst) {
            return true;
        }
        self.child.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}
