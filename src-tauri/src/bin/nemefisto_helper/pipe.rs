//! Named pipe сервер с line-delimited JSON-RPC.
//!
//! Слушает `\\.\pipe\nemefisto-helper`. Каждое подключение обрабатывается
//! в отдельной задаче. Клиент шлёт `Request` (одна строка JSON, `\n`-терминатор),
//! получает `Response` (одна строка JSON, `\n`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use super::dispatch;
use super::protocol::{Request, Response, PIPE_NAME};
use super::security::PipeSecurity;

/// Создать NamedPipeServer с permissive ACL чтобы user-mode клиенты могли
/// подключаться к pipe сервиса от SYSTEM.
fn create_pipe(first_instance: bool) -> Result<NamedPipeServer> {
    let mut security = PipeSecurity::permissive();
    let attrs = security.as_attrs_ptr();
    let mut opts = ServerOptions::new();
    if first_instance {
        opts.first_pipe_instance(true);
    }
    let server = unsafe {
        opts.create_with_security_attributes_raw(PIPE_NAME, attrs)
            .with_context(|| format!("CreateNamedPipe {PIPE_NAME}"))?
    };
    // security должен жить как минимум до момента CreateNamedPipe — он живёт
    // до конца этой функции, что больше требуемого. Дроп безопасен.
    drop(security);
    Ok(server)
}

/// Запускает сервер до тех пор, пока `shutdown` не станет true.
/// При установке флага текущий accept будет прерван (через select! с tick).
pub async fn run_pipe_server(shutdown: Arc<AtomicBool>) -> Result<()> {
    eprintln!("[helper-pipe] слушаем {PIPE_NAME}");

    let mut server = create_pipe(true)?;

    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        tokio::select! {
            connect_result = server.connect() => {
                connect_result.context("ошибка accept на pipe")?;

                // Передаём текущий instance клиенту, создаём новый для следующего.
                let connected = server;
                server = create_pipe(false)?;

                tokio::spawn(handle_client(connected));
            }
            _ = tick.tick() => {
                // Просто проверяем shutdown
            }
        }
    }

    eprintln!("[helper-pipe] остановка по shutdown");
    Ok(())
}

async fn handle_client(pipe: NamedPipeServer) {
    if let Err(e) = handle_client_inner(pipe).await {
        eprintln!("[helper-pipe] клиент отключён с ошибкой: {e:#}");
    }
}

async fn handle_client_inner(pipe: NamedPipeServer) -> Result<()> {
    let (read_half, mut write_half) = tokio::io::split(pipe);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF — клиент закрыл соединение
            return Ok(());
        }

        let request: Request = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(format!("невалидный JSON-запрос: {e}"));
                send_response(&mut write_half, &resp).await?;
                continue;
            }
        };

        let response = dispatch::handle(request).await;
        send_response(&mut write_half, &response).await?;
    }
}

async fn send_response<W: AsyncWriteExt + Unpin>(w: &mut W, resp: &Response) -> Result<()> {
    let mut bytes = serde_json::to_vec(resp)?;
    bytes.push(b'\n');
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}
