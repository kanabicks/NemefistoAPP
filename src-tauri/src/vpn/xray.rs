//! Управление процессом Xray-core (sidecar).
//!
//! Принимает готовый JSON-конфиг, записывает во временный файл и запускает Xray.
//! При завершении процесса (Terminated) сбрасывает системный прокси.

use std::fs::File;
use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

/// Глобальный state Xray sidecar.
///
/// Кроме самого `CommandChild` храним `pid` запущенного процесса, чтобы
/// при гонке (быстрый перезапуск) старый listener не вытер ссылку на новый.
pub struct XrayState {
    child: Mutex<Option<CommandChild>>,
    current_pid: Mutex<Option<u32>>,
    pub socks_port: Mutex<u16>,
    pub http_port: Mutex<u16>,
}

impl XrayState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
            current_pid: Mutex::new(None),
            socks_port: Mutex::new(1080),
            http_port: Mutex::new(1087),
        }
    }

    /// Запустить Xray с указанным JSON-конфигом.
    ///
    /// Если Xray уже запущен — останавливает его перед перезапуском.
    /// Конфиг сохраняется в %TEMP%\NemefistoVPN\xray-config.json.
    pub fn start_with_config(
        &self,
        app: &AppHandle,
        config_json: &Value,
        socks_port: u16,
        http_port: u16,
    ) -> Result<(), String> {
        // Останавливаем предыдущий процесс
        self.stop()?;

        // Пишем конфиг во временный файл
        let tmp_dir = std::env::temp_dir().join("NemefistoVPN");
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("не удалось создать %TEMP%\\NemefistoVPN: {e}"))?;
        let config_path = tmp_dir.join("xray-config.json");
        let config_str = serde_json::to_string_pretty(config_json)
            .map_err(|e| format!("сериализация конфига: {e}"))?;
        std::fs::write(&config_path, &config_str)
            .map_err(|e| format!("запись конфига: {e}"))?;

        let config_path_str = config_path
            .to_str()
            .ok_or_else(|| "путь к конфигу содержит не-UTF-8 символы".to_string())?;

        // Файл для tee stderr — гарантированно ловит последнюю строку перед смертью
        let stderr_log_path = tmp_dir.join("xray-stderr.log");
        let stderr_log: Arc<Mutex<File>> = Arc::new(Mutex::new(
            File::create(&stderr_log_path)
                .map_err(|e| format!("создание файла stderr-лога: {e}"))?,
        ));

        let (mut rx, child) = app
            .shell()
            .sidecar("xray")
            .map_err(|e| format!("sidecar xray не зарегистрирован: {e}"))?
            .args(["-config", config_path_str])
            .spawn()
            .map_err(|e| format!("не удалось запустить xray: {e}"))?;

        let my_pid = child.pid();
        eprintln!("[xray] запущен pid={my_pid}, stderr-лог: {stderr_log_path:?}");

        {
            let mut g = self.child.lock().map_err(|e| format!("mutex: {e}"))?;
            *g = Some(child);
        }
        *self
            .current_pid
            .lock()
            .map_err(|e| format!("mutex: {e}"))? = Some(my_pid);
        *self.socks_port.lock().map_err(|e| format!("mutex: {e}"))? = socks_port;
        *self.http_port.lock().map_err(|e| format!("mutex: {e}"))? = http_port;

        let app_handle = app.clone();
        let stderr_log_clone = stderr_log.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        eprintln!("[xray:out] {}", String::from_utf8_lossy(&line));
                    }
                    CommandEvent::Stderr(line) => {
                        let s = String::from_utf8_lossy(&line);
                        eprintln!("[xray:err] {s}");
                        if let Ok(mut f) = stderr_log_clone.lock() {
                            let _ = writeln!(f, "{s}");
                            let _ = f.flush();
                        }
                    }
                    CommandEvent::Terminated(payload) => {
                        eprintln!(
                            "[xray] завершён pid={my_pid}: code={:?}, signal={:?}",
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
                        // Защита от race: чистим state только если pid совпадает с нашим.
                        // Иначе мог уже быть запущен новый процесс — не трогаем его.
                        let state = app_handle.state::<XrayState>();
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
                            let _ = crate::platform::proxy::clear_system_proxy();
                        } else {
                            eprintln!(
                                "[xray] pid={my_pid} устаревший — не трогаем state нового процесса"
                            );
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
        let mut g = self.child.lock().map_err(|e| format!("mutex: {e}"))?;
        if let Some(child) = g.take() {
            let pid = child.pid();
            eprintln!("[xray] kill pid={pid} (явный stop)");
            child.kill().map_err(|e| format!("kill xray: {e}"))?;
        }
        if let Ok(mut g) = self.current_pid.lock() {
            *g = None;
        }
        Ok(())
    }

    /// Запущен ли Xray прямо сейчас.
    pub fn is_running(&self) -> bool {
        self.child.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Порты, на которых сейчас слушает Xray.
    pub fn current_ports(&self) -> (u16, u16) {
        let s = self.socks_port.lock().map(|g| *g).unwrap_or(1080);
        let h = self.http_port.lock().map(|g| *g).unwrap_or(1087);
        (s, h)
    }
}

/// Найти свободный порт, начиная с `start` (проверяет до 20 вариантов).
pub fn find_free_port(start: u16) -> u16 {
    (start..start.saturating_add(20))
        .find(|&p| TcpListener::bind(("127.0.0.1", p)).is_ok())
        .unwrap_or(start)
}

/// Псевдослучайный порт в высоком диапазоне `[30000, 60000)`.
///
/// Используется как стартовая точка для `find_free_port` при поднятии
/// inbound'ов Xray. Цель — защита от детекта VPN-клиента сторонним
/// процессом, сканирующим стандартные SOCKS-порты (7890, 1080, 1087);
/// см. этап 9.H в CLAUDE.md и https://habr.com/ru/news/1020902/.
///
/// Источник «случайности» — наносекунды текущего времени; криптостойкость
/// не нужна, достаточно того что от запуска к запуску значение разное и
/// для стороннего процесса непредсказуемо без активного полного сканирования.
pub fn random_high_port() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0);
    30000 + (seed % 30000) as u16
}
