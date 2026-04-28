//! TCP-connect ping для оценки задержки до сервера.
//!
//! Не использует ICMP (требует raw socket / прав администратора). Вместо
//! этого замеряет время установления TCP-соединения к `host:port` сервера,
//! что даёт практически релевантную метрику для VPN-подключения.

use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::ProxyEntry;

const PING_TIMEOUT_MS: u64 = 2500;

/// TCP-connect-ping. Возвращает время в миллисекундах или `None`, если
/// сервер не отвечает в течение `PING_TIMEOUT_MS`.
pub async fn tcp_ping(host: &str, port: u16) -> Option<u32> {
    let addr = format!("{host}:{port}");
    let start = Instant::now();
    match timeout(Duration::from_millis(PING_TIMEOUT_MS), TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => Some(start.elapsed().as_millis() as u32),
        _ => None,
    }
}

/// Извлечь host/port из ProxyEntry для пинга.
///
/// Для обычных серверов берётся `entry.server` / `entry.port`.
/// Для `xray-json` ищется первый outbound с тегом, начинающимся на `proxy`,
/// и берётся адрес из его настроек (vnext для VLESS/VMess, servers для Trojan/SS).
pub fn extract_target(entry: &ProxyEntry) -> Option<(String, u16)> {
    if entry.protocol != "xray-json" {
        if entry.server.is_empty() || entry.port == 0 {
            return None;
        }
        return Some((entry.server.clone(), entry.port));
    }

    let outbounds = entry.raw.get("outbounds")?.as_array()?;
    for ob in outbounds {
        let tag = ob.get("tag").and_then(Value::as_str).unwrap_or("");
        if !tag.starts_with("proxy") {
            continue;
        }
        let proto = ob.get("protocol").and_then(Value::as_str).unwrap_or("");
        let settings = ob.get("settings")?;

        let target = match proto {
            "vless" | "vmess" => {
                let first = settings.get("vnext")?.as_array()?.first()?;
                let host = first.get("address")?.as_str()?;
                let port = first.get("port")?.as_u64()? as u16;
                Some((host.to_string(), port))
            }
            "trojan" | "shadowsocks" => {
                let first = settings.get("servers")?.as_array()?.first()?;
                let host = first.get("address")?.as_str()?;
                let port = first.get("port")?.as_u64()? as u16;
                Some((host.to_string(), port))
            }
            _ => None,
        };
        if target.is_some() {
            return target;
        }
    }
    None
}

/// Пингует один ProxyEntry. Возвращает None если адрес не извлекается
/// или если сервер не ответил в timeout.
pub async fn ping_entry(entry: &ProxyEntry) -> Option<u32> {
    let (host, port) = extract_target(entry)?;
    tcp_ping(&host, port).await
}
