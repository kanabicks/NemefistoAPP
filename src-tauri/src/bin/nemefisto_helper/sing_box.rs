//! SYSTEM-spawned sing-box для built-in TUN-режима.
//!
//! WinTUN требует админских прав на `CreateAdapter`. Tauri-main работает
//! как обычный user (без UAC-elevation на запуск приложения), поэтому
//! не может запустить sing-box с правом создания адаптера, если конфиг
//! содержит `inbound: tun`. Helper-сервис работает как `LocalSystem` —
//! у него этих прав достаточно.
//!
//! Архитектурно зеркалит `mihomo.rs`: один глобальный `STATE`, один
//! активный sing-box за раз, stdout/stderr перенаправляются в
//! `C:\ProgramData\NemefistoVPN\sing-box.log` (SYSTEM-write,
//! admin-user-read для диагностики).
//!
//! Tauri-main коммуницирует с sing-box через `mixed`-inbound на
//! 127.0.0.1 (loopback виден SYSTEM и user'у одинаково), и (если включен)
//! Clash-API на отдельном порту — для warmup'а / leak-test / smart-failover.

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

/// Запустить sing-box с указанным конфигом. Stdout/stderr → файл-лог.
///
/// При повторном вызове пока sing-box жив — bail. Tauri-main должен
/// сначала остановить предыдущий через `stop()`. Это идёт по логике
/// connect/disconnect: один активный движок за раз.
pub async fn start(
    config_path: &str,
    exe_path: &str,
    data_dir: &str,
) -> Result<()> {
    let mut g = STATE.lock().await;
    if g.is_some() {
        bail!("sing-box уже запущен (используйте singbox_stop сначала)");
    }

    let exe = Path::new(exe_path);
    if !exe.is_file() {
        bail!("sing-box не найден: {exe_path}");
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
    let log_path = log_dir.join("sing-box.log");
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("создание {}", log_path.display()))?;
    let log_clone = log_file
        .try_clone()
        .context("клонирование лог-файла для stderr")?;

    // sing-box CLI: `sing-box run -c <config> -D <working-dir>`.
    // -D устанавливает рабочую директорию для cache/rule-set.
    let mut cmd = Command::new(exe);
    cmd.args(["run", "-c", config_path, "-D", data_dir])
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

    let child = cmd.spawn().context("spawn sing-box")?;
    let pid = child.id().unwrap_or(0);
    eprintln!(
        "[helper-singbox] запущен pid={pid}, лог: {}",
        log_path.display()
    );

    *g = Some(State { child, pid });
    Ok(())
}

/// Остановить sing-box. Идемпотентно: если не запущен — Ok.
///
/// sing-box при graceful kill (SIGTERM аналог на Windows — но у нас
/// `child.kill()` использует TerminateProcess) всё равно успевает
/// убрать свой WinTUN-адаптер благодаря `auto_route` cleanup-логике
/// внутри Go-runtime. Если kill жёсткий — driver сам отвалится через
/// несколько секунд + наш `cleanup_orphan_resources` подберёт остатки
/// на следующем старте helper'а.
pub async fn stop() -> Result<()> {
    let mut g = STATE.lock().await;
    let state = match g.take() {
        Some(s) => s,
        None => return Ok(()),
    };
    let pid = state.pid;
    let mut child = state.child;
    eprintln!("[helper-singbox] kill pid={pid}");
    if let Err(e) = child.kill().await {
        eprintln!("[helper-singbox] kill failed: {e}");
    }
    // wait — освобождаем zombie. Не блокируем долго: sing-box обычно
    // умирает за миллисекунды, иначе ОС всё равно убьёт.
    let wait = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        child.wait(),
    )
    .await;
    match wait {
        Ok(Ok(status)) => {
            eprintln!("[helper-singbox] pid={pid} завершён со статусом {status}");
        }
        Ok(Err(e)) => {
            eprintln!("[helper-singbox] wait error: {e}");
        }
        Err(_) => {
            return Err(anyhow!("sing-box не остановился за 3 секунды"));
        }
    }
    Ok(())
}

/// Запущен ли sing-box helper'ом сейчас. Используется для диагностики.
#[allow(dead_code)]
pub async fn is_running() -> bool {
    STATE.lock().await.is_some()
}
