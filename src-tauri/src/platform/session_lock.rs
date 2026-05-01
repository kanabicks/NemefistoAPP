//! Lockfile для детекции краха предыдущей сессии.
//!
//! При старте main app пишем `%LOCALAPPDATA%\NemefistoVPN\session.lock` с
//! текущим PID. При **clean exit** (через WindowEvent::Destroyed) удаляем
//! файл. Если на старте видим существующий lockfile — проверяем жив ли тот
//! PID:
//!   - не жив / другой процесс → прошлая сессия упала, нужен recovery
//!   - жив и это `vpn-client.exe` → две инстанции, абортим текущую
//!     (Tauri single-instance защита; на практике сюда мы не доходим
//!     благодаря deep-link плагину, но lockfile это финальный gate).
//!
//! При kill -9 / shutdown system / hardware crash файл остаётся stale —
//! это нормальное поведение и именно его мы и ловим: следующий старт
//! автоматически запускает self-healing.

use std::path::PathBuf;

const LOCK_DIR: &str = "NemefistoVPN";
const LOCK_FILE: &str = "session.lock";
/// Имя нашего основного exe — используем для отличия «наш живой PID»
/// от «другой процесс с тем же PID» (после reboot/wrap-around).
const OUR_EXE_NAME: &str = "vpn-client.exe";

fn lock_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(local).join(LOCK_DIR).join(LOCK_FILE))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
        Some(base.join(LOCK_DIR).join(LOCK_FILE))
    }
}

/// Результат `acquire`. Используется на старте чтобы понять надо ли
/// запускать recovery sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// Lockfile отсутствовал — чистый запуск.
    FreshStart,
    /// Был stale lockfile (PID мёртв / процесс не наш) — прошлая сессия
    /// упала, надо запустить self-healing.
    PreviousSessionCrashed,
}

/// Захватить lock. Пишет наш PID в файл. Возвращает информацию о том,
/// что было до нас.
///
/// Не блокирует запуск ни в каком случае — мы не используем lockfile
/// как mutex, только как crash-detection. Двойной запуск контролируется
/// отдельно (Tauri single-instance / deep-link plugin).
pub fn acquire() -> AcquireOutcome {
    let Some(path) = lock_path() else {
        return AcquireOutcome::FreshStart;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let outcome = match std::fs::read_to_string(&path) {
        Ok(content) => {
            let prev_pid: Option<u32> = content.trim().lines().next().and_then(|s| s.parse().ok());
            match prev_pid {
                Some(pid) if is_our_process_alive(pid) => {
                    // Жив другой наш main — мы вторая копия. Считаем это
                    // fresh start (мы скоро завершимся через single-instance,
                    // или это нормальная второй запуск после restart).
                    AcquireOutcome::FreshStart
                }
                _ => AcquireOutcome::PreviousSessionCrashed,
            }
        }
        Err(_) => AcquireOutcome::FreshStart,
    };

    let our_pid = std::process::id();
    let _ = std::fs::write(&path, format!("{our_pid}\n"));
    outcome
}

/// Удалить lockfile. Вызывается при clean exit (WindowEvent::Destroyed).
/// Best-effort: ошибки игнорим (всё равно next-start обнаружит и починит).
pub fn release() {
    if let Some(path) = lock_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Проверка: жив ли процесс с указанным PID и это наш `vpn-client.exe`.
///
/// Через `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + базовое имя
/// модуля. Если процесс умер / занят другим bin'ом — возвращаем false
/// (значит lockfile stale, прошлая сессия упала).
#[cfg(windows)]
fn is_our_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::ProcessStatus::K32GetModuleBaseNameW;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 || pid == std::process::id() {
        return false;
    }
    unsafe {
        let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if h.is_null() {
            return false;
        }
        let mut name_buf = [0u16; 260];
        let len = K32GetModuleBaseNameW(
            h,
            std::ptr::null_mut(),
            name_buf.as_mut_ptr(),
            name_buf.len() as u32,
        );
        CloseHandle(h);
        if len == 0 {
            return false;
        }
        let name = String::from_utf16_lossy(&name_buf[..len as usize]).to_lowercase();
        name == OUR_EXE_NAME
    }
}

#[cfg(not(windows))]
fn is_our_process_alive(_pid: u32) -> bool {
    false
}
