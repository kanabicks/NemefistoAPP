//! Nemefisto VPN Helper — Windows-сервис, выполняющий привилегированные
//! операции от имени SYSTEM: создание TUN-интерфейса (через WinTUN),
//! настройка системного routing-а, запуск tun2socks-процесса.
//!
//! User-mode Tauri-приложение общается с этим helper-ом через named pipe
//! `\\.\pipe\nemefisto-helper` line-delimited JSON-RPC протоколом.
//!
//! CLI:
//!   nemefisto-helper install      — установить и запустить сервис (нужен UAC)
//!   nemefisto-helper uninstall    — остановить и удалить сервис (нужен UAC)
//!   nemefisto-helper service      — точка входа SCM, не вызывать руками
//!   nemefisto-helper status       — диагностический ping в pipe

#[cfg(windows)]
mod nemefisto_helper {
    pub mod crash_dumps;
    pub mod dispatch;
    pub mod firewall;
    pub mod helper_log;
    pub mod mihomo;
    pub mod pipe;
    pub mod protocol;
    pub mod routing;
    pub mod security;
    pub mod service;
    pub mod sing_box;
    pub mod tun;
    pub mod wfp;
}

#[cfg(windows)]
fn main() {
    // 14.C: panic-hook первой строкой — даже если service::install
    // паникует, мы это запишем в файл.
    nemefisto_helper::crash_dumps::install_panic_hook();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    let result: anyhow::Result<()> = match cmd {
        "install" => nemefisto_helper::service::install(),
        "uninstall" => nemefisto_helper::service::uninstall(),
        "service" => nemefisto_helper::service::run_as_service(),
        "debug" => run_debug_foreground(),
        "status" => match status_check() {
            Ok(version) => {
                println!("сервис отвечает, версия: {version}");
                Ok(())
            }
            Err(e) => Err(e),
        },
        // 13.D EMERGENCY: восстанавливает интернет если kill-switch
        // фильтры остались висеть (helper не убрал их при crash, или
        // DYNAMIC не сработал). Не требует запущенного сервиса —
        // открывает свой WFP-engine, удаляет наш provider+sublayer
        // каскадно (вместе со всеми filter'ами).
        // Запускать ОТ АДМИНА:
        //   & "C:\path\to\nemefisto-helper.exe" killswitch-cleanup
        "killswitch-cleanup" => {
            match nemefisto_helper::wfp::cleanup_provider() {
                Ok(()) => {
                    println!("✓ WFP kill-switch фильтры удалены, интернет восстановлен");
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ => {
            print_usage();
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("ошибка: {e:#}");
        std::process::exit(1);
    }
}

/// Foreground-режим: pipe-сервер крутится прямо в этой консоли без
/// регистрации Windows-сервиса. Нужны admin-права (для tun2socks/routes).
/// Ctrl+C — корректное завершение через shutdown-флаг.
#[cfg(windows)]
fn run_debug_foreground() -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_for_handler = shutdown.clone();
    ctrlc_or_warn(move || {
        eprintln!("\n[helper-debug] получен Ctrl+C, выход…");
        shutdown_for_handler.store(true, Ordering::SeqCst);
    });

    eprintln!("[helper-debug] foreground-режим (нет регистрации сервиса)");
    eprintln!("[helper-debug] Ctrl+C для выхода");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        // 13.D: то же что в service.rs — cleanup orphan-фильтров
        // на старте debug-режима (для тестов вручную).
        if let Err(err) = nemefisto_helper::firewall::cleanup_on_startup().await {
            eprintln!("[helper-debug] startup cleanup error: {err}");
        }
        nemefisto_helper::pipe::run_pipe_server(shutdown).await
    })?;
    Ok(())
}

/// Простейший Ctrl+C handler через windows-sys — без новой зависимости.
#[cfg(windows)]
fn ctrlc_or_warn<F: FnMut() + Send + 'static>(handler: F) {
    use std::sync::Mutex;
    static HANDLER: Mutex<Option<Box<dyn FnMut() + Send>>> = Mutex::new(None);
    *HANDLER.lock().unwrap() = Some(Box::new(handler));

    unsafe extern "system" fn ctrl_handler(_: u32) -> i32 {
        if let Ok(mut g) = HANDLER.lock() {
            if let Some(h) = g.as_mut() {
                h();
            }
        }
        1 // TRUE — обработали
    }

    unsafe {
        let _ = windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(ctrl_handler), 1);
    }
}

#[cfg(windows)]
fn status_check() -> anyhow::Result<String> {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pipe = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(nemefisto_helper::protocol::PIPE_NAME)
        {
            Ok(f) => break f,
            Err(e) => {
                if Instant::now() >= deadline {
                    anyhow::bail!("не удалось подключиться к pipe: {e}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };

    // ping
    pipe.write_all(b"{\"cmd\":\"ping\"}\n")?;
    let mut buf = [0u8; 1024];
    let n = pipe.read(&mut buf)?;
    let resp: serde_json::Value = serde_json::from_slice(&buf[..n])?;
    if resp.get("result").and_then(|v| v.as_str()) != Some("pong") {
        anyhow::bail!("ожидали pong, получили {resp}");
    }

    // version
    pipe.write_all(b"{\"cmd\":\"version\"}\n")?;
    let n = pipe.read(&mut buf)?;
    let resp: serde_json::Value = serde_json::from_slice(&buf[..n])?;
    Ok(resp
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("нет поля version: {resp}"))?
        .to_string())
}

#[cfg(windows)]
fn print_usage() {
    eprintln!("nemefisto-helper — Windows-сервис для управления TUN-режимом");
    eprintln!();
    eprintln!("usage:");
    eprintln!("  nemefisto-helper install     установить и запустить сервис");
    eprintln!("  nemefisto-helper uninstall   остановить и удалить сервис");
    eprintln!("  nemefisto-helper service     (внутренняя — вызывается SCM)");
    eprintln!("  nemefisto-helper debug       foreground-режим для отладки");
    eprintln!("  nemefisto-helper status      проверить, что сервис отвечает");
    eprintln!(
        "  nemefisto-helper killswitch-cleanup  EMERGENCY: убрать WFP-фильтры если \
         kill-switch завис и интернет заблокирован"
    );
}

#[cfg(not(windows))]
fn main() {
    eprintln!("nemefisto-helper поддерживается только на Windows");
    std::process::exit(1);
}
