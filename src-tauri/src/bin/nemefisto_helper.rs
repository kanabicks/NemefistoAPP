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
    pub mod dispatch;
    pub mod pipe;
    pub mod protocol;
    pub mod routing;
    pub mod security;
    pub mod service;
    pub mod tun;
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    let result: anyhow::Result<()> = match cmd {
        "install" => nemefisto_helper::service::install(),
        "uninstall" => nemefisto_helper::service::uninstall(),
        "service" => nemefisto_helper::service::run_as_service(),
        "status" => match status_check() {
            Ok(version) => {
                println!("сервис отвечает, версия: {version}");
                Ok(())
            }
            Err(e) => Err(e),
        },
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
    eprintln!("  nemefisto-helper status      проверить, что сервис отвечает");
}

#[cfg(not(windows))]
fn main() {
    eprintln!("nemefisto-helper поддерживается только на Windows");
    std::process::exit(1);
}
