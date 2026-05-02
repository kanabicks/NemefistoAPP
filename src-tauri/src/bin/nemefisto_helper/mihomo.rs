//! 13.L: SYSTEM-spawned mihomo для built-in TUN-режима.
//!
//! WinTUN требует админских прав на `CreateAdapter`. Tauri-main работает
//! как обычный user (без UAC-elevation на запуск приложения), поэтому
//! не может запустить mihomo с правом создания адаптера. Helper-сервис
//! работает как `LocalSystem` — у него этих прав более чем достаточно.
//!
//! Этот модуль предоставляет helper'у возможность spawn'ить mihomo
//! по запросу от Tauri-main и держать процесс под мьютексом до явного
//! `MihomoStop`. Stdout/stderr перенаправляются в
//! `C:\ProgramData\NemefistoVPN\mihomo.log`.
//!
//! Tauri-main коммуницирует с mihomo через external-controller на
//! 127.0.0.1 (loopback виден SYSTEM и user'у одинаково), а live-status
//! проверяет через ping этого endpoint'а или через `mihomo_proxies`
//! IPC-команду.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[allow(dead_code)]
struct State {
    child: Child,
    /// PID для логирования и идемпотентного stop'а — если процесс уже
    /// сам выключился, мы не падаем.
    pid: u32,
}

static STATE: Mutex<Option<State>> = Mutex::const_new(None);

/// Запустить mihomo с указанным конфигом. Stdout/stderr → файл-лог.
///
/// При повторном вызове пока mihomo жив — bail. Tauri-main должен
/// сначала остановить предыдущий через `stop()`. Это идёт по логике
/// connect/disconnect: один активный движок за раз.
pub async fn start(
    config_path: &str,
    exe_path: &str,
    data_dir: &str,
) -> Result<()> {
    let mut g = STATE.lock().await;
    if g.is_some() {
        bail!("mihomo уже запущен (используйте mihomo_stop сначала)");
    }

    let exe = Path::new(exe_path);
    if !exe.is_file() {
        bail!("mihomo не найден: {exe_path}");
    }
    let config = Path::new(config_path);
    if !config.is_file() {
        bail!("конфиг не найден: {config_path}");
    }
    std::fs::create_dir_all(data_dir).context("создание data-dir")?;

    // Лог в ProgramData — туда у SYSTEM есть write-access, и admin-user
    // может прочитать без UAC. Перезаписываем при каждом start (старые
    // логи не нужны, хранение в файлах рваное).
    let log_dir = PathBuf::from(r"C:\ProgramData\NemefistoVPN");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("mihomo.log");
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("создание {}", log_path.display()))?;
    let log_clone = log_file
        .try_clone()
        .context("клонирование лог-файла для stderr")?;

    let mut cmd = Command::new(exe);
    cmd.args(["-f", config_path, "-d", data_dir])
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_clone))
        .kill_on_drop(true);

    // CREATE_NO_WINDOW — без мигающего консольного окна на старте.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = cmd.spawn().context("spawn mihomo")?;
    let pid = child.id().unwrap_or(0);
    eprintln!(
        "[helper-mihomo] запущен pid={pid}, лог: {}",
        log_path.display()
    );

    *g = Some(State { child, pid });
    Ok(())
}

/// Остановить mihomo. Идемпотентно: если не запущен — Ok.
///
/// Mihomo при graceful kill сам убирает свой WinTUN-адаптер и
/// маршруты (auto-route добавил — auto-route и удалит). Если kill
/// жёсткий (SIGKILL-аналог), driver сам отвалится через несколько
/// секунд + наш `cleanup_orphan_resources` подберёт остатки на
/// следующем старте helper'а.
pub async fn stop() -> Result<()> {
    let mut g = STATE.lock().await;
    let state = match g.take() {
        Some(s) => s,
        None => return Ok(()),
    };
    let pid = state.pid;
    let mut child = state.child;
    eprintln!("[helper-mihomo] kill pid={pid}");
    if let Err(e) = child.kill().await {
        eprintln!("[helper-mihomo] kill failed: {e}");
    }
    // wait — освобождаем zombie. Не блокируем долго: mihomo обычно
    // умирает за миллисекунды, иначе ОС всё равно убьёт.
    let wait = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        child.wait(),
    )
    .await;
    match wait {
        Ok(Ok(status)) => {
            eprintln!("[helper-mihomo] pid={pid} завершён со статусом {status}");
        }
        Ok(Err(e)) => {
            eprintln!("[helper-mihomo] wait error: {e}");
        }
        Err(_) => {
            return Err(anyhow!("mihomo не остановился за 3 секунды"));
        }
    }
    Ok(())
}

/// Запущен ли mihomo helper'ом сейчас. Используется для диагностики
/// (например, при cleanup orphan-ресурсов на старте сервиса).
#[allow(dead_code)]
pub async fn is_running() -> bool {
    STATE.lock().await.is_some()
}
