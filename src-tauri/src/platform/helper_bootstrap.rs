//! Авто-обнаружение и авто-установка helper-сервиса.
//!
//! Цель: пользователь никогда не открывает PowerShell вручную чтобы
//! установить сервис. При первом TUN-подключении мы:
//!   1. Проверяем, отвечает ли уже helper по named pipe.
//!   2. Если нет — находим `nemefisto-helper.exe`, запускаем его с
//!      аргументом `install` через `ShellExecuteW` с verb `runas`
//!      (UAC-запрос «разрешить от имени админа»).
//!   3. Ждём пока сервис поднимется и начнёт отвечать на ping.
//!
//! Сервис ставится с типом `AutoStart` — после установки он переживает
//! перезагрузку и больше UAC не требует.
//!
//! Если пользователь нажимает «Нет» в UAC — `ShellExecuteW` возвращает
//! `SE_ERR_ACCESSDENIED` (5), мы возвращаем понятную ошибку.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use super::helper_client;

const HELPER_FILENAME: &str = "nemefisto-helper.exe";
/// Имя helper'а в bundle Tauri. ExternalBin копирует sidecar с triplet-
/// суффиксом, отделить который мы не контролируем (зависит от версии Tauri).
const HELPER_FILENAME_TRIPLET: &str = "nemefisto-helper-x86_64-pc-windows-msvc.exe";
const PING_TIMEOUT_AFTER_INSTALL: Duration = Duration::from_secs(20);
const PING_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Гарантирует что helper отвечает по pipe-у. Если уже отвечает — мгновенно
/// возвращает Ok. Если нет — запускает auto-install через UAC и ждёт пока
/// сервис начнёт отвечать.
///
/// Возможные исходы:
/// - `Ok(())` — helper доступен (был доступен изначально, или поставлен).
/// - `Err(...)` — helper не найден в файловой системе, либо UAC отменён,
///   либо сервис установился но не отвечает за 20 секунд.
pub async fn ensure_running() -> Result<()> {
    // 1. Быстрая проверка — может уже работает
    if helper_client::ping().await.is_ok() {
        return Ok(());
    }

    // 2. Найти helper.exe
    let helper = resolve_helper_path()
        .ok_or_else(|| anyhow::anyhow!(
            "{HELPER_FILENAME} не найден ни рядом с приложением, ни в target/{{debug,release}}/"
        ))?;

    eprintln!(
        "[helper-bootstrap] helper не отвечает, запускаю install через UAC: {}",
        helper.display()
    );

    // 3. Запуск с UAC
    spawn_elevated(&helper, "install")?;

    // 4. Ждём пока сервис поднимется и начнёт отвечать
    let deadline = Instant::now() + PING_TIMEOUT_AFTER_INSTALL;
    while Instant::now() < deadline {
        tokio::time::sleep(PING_POLL_INTERVAL).await;
        if helper_client::ping().await.is_ok() {
            eprintln!("[helper-bootstrap] helper отвечает, установка успешна");
            return Ok(());
        }
    }

    bail!(
        "helper-сервис установился, но не отозвался за {}с. Проверьте \
         services.msc → NemefistoHelper",
        PING_TIMEOUT_AFTER_INSTALL.as_secs()
    )
}

/// Найти `nemefisto-helper.exe` в нескольких возможных локациях:
///   1. `<exe-dir>/nemefisto-helper.exe`            — dev (target/debug,
///                                                    target/release)
///                                                    или prod если Tauri
///                                                    стрипает triplet;
///   2. `<exe-dir>/nemefisto-helper-<triplet>.exe`  — prod если Tauri
///                                                    оставляет triplet
///                                                    после bundle;
///   3. `<exe-dir>/resources/...`                   — fallback на случай
///                                                    нестандартного
///                                                    расположения.
fn resolve_helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let candidates = [
        exe_dir.join(HELPER_FILENAME),
        exe_dir.join(HELPER_FILENAME_TRIPLET),
        exe_dir.join("resources").join(HELPER_FILENAME),
        exe_dir.join("resources").join(HELPER_FILENAME_TRIPLET),
    ];

    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Запустить процесс с правами администратора через ShellExecuteW + verb=runas.
/// Не ждёт его завершения — UAC blocking управляется ОС, мы продолжаем
/// после клика «Да» / «Нет».
#[cfg(windows)]
fn spawn_elevated(exe: &Path, arg: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn wide_path(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    let verb = wide("runas");
    let file = wide_path(exe);
    let params = wide(arg);

    // ShellExecuteW возвращает HINSTANCE — fake-handle, > 32 = успех,
    // ≤ 32 = код ошибки.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_HIDE as i32,
        )
    };

    let code = result as isize;
    if code > 32 {
        Ok(())
    } else if code == 5 {
        // SE_ERR_ACCESSDENIED — пользователь нажал «Нет» в UAC
        bail!("установка helper-сервиса отменена пользователем (UAC)")
    } else {
        bail!("ShellExecuteW вернул код {code}")
    }
}

#[cfg(not(windows))]
fn spawn_elevated(_exe: &Path, _arg: &str) -> Result<()> {
    bail!("auto-install helper поддерживается только на Windows")
}
