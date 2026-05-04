//! HTTP-/TCP-ping через активное соединение для теста целостности
//! туннеля. Дополняет `ping.rs` (TCP до VPN-сервера для server-list)
//! и `leak_test.rs` (полная проверка с GeoIP + DNS leak).
//!
//! Запросы идут через локальный SOCKS5 inbound если задан `socks_port`
//! (proxy-режим), иначе через system route (TUN-режим, где route
//! и так через VPN). Для TCP-метода `socks_port` игнорируется —
//! TCP-ping работает напрямую к URL (нужен только для оценки RTT
//! до сервера URL'а).

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Метод ping'а. Сериализуется как kebab-case для UI.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PingMethod {
    /// TCP-connect к URL'овому host:port. Не использует прокси —
    /// напрямую к URL'у. Для TUN-mode совпадает с system route, для
    /// proxy-mode идёт мимо туннеля (нужен только как baseline).
    Tcp,
    /// HTTP GET через прокси (если `socks_port` задан) или напрямую.
    /// Полная цепочка: TCP → TLS → HTTP request → response.
    HttpGet,
    /// HTTP HEAD — то же что GET, но без body (быстрее, меньше трафика).
    HttpHead,
}

/// Результат ping'а. Сериализуется в JSON для фронта.
#[derive(Debug, Clone, Serialize)]
pub struct PingResult {
    /// Латентность в миллисекундах. `None` если timeout / ошибка.
    pub latency_ms: Option<u32>,
    /// HTTP status code (для GET/HEAD). `None` если TCP / ошибка.
    pub status: Option<u16>,
    /// Текстовое описание ошибки если `latency_ms.is_none()`.
    pub error: Option<String>,
    /// Использовался ли SOCKS5 прокси (для отображения «через VPN»
    /// vs «напрямую» в UI).
    pub via_proxy: bool,
}

/// Замерить ping выбранным методом.
///
/// `socks_port` — если задан и метод HTTP-*, запрос идёт через
/// `socks5h://127.0.0.1:{port}`. Для TCP игнорируется.
///
/// `url` — целевой URL. Для TCP парсится только host:port,
/// для HTTP — используется целиком.
///
/// `timeout_secs` — общий таймаут на все этапы (DNS+TCP+TLS+HTTP).
pub async fn ping(
    method: PingMethod,
    url: &str,
    socks_port: Option<u16>,
    timeout_secs: u32,
) -> PingResult {
    let timeout_dur = Duration::from_secs(timeout_secs.max(1) as u64);
    match method {
        PingMethod::Tcp => tcp_ping(url, timeout_dur).await,
        PingMethod::HttpGet => http_ping(url, "GET", socks_port, timeout_dur).await,
        PingMethod::HttpHead => http_ping(url, "HEAD", socks_port, timeout_dur).await,
    }
}

async fn tcp_ping(url: &str, timeout_dur: Duration) -> PingResult {
    let parsed = match parse_host_port(url) {
        Ok(v) => v,
        Err(e) => {
            return PingResult {
                latency_ms: None,
                status: None,
                error: Some(format!("неверный URL: {e:#}")),
                via_proxy: false,
            };
        }
    };
    let (host, port) = parsed;
    let addr = format!("{host}:{port}");

    let start = Instant::now();
    let res = timeout(timeout_dur, TcpStream::connect(&addr)).await;
    let elapsed_ms = start.elapsed().as_millis() as u32;
    match res {
        Ok(Ok(_)) => PingResult {
            latency_ms: Some(elapsed_ms),
            status: None,
            error: None,
            via_proxy: false,
        },
        Ok(Err(e)) => PingResult {
            latency_ms: None,
            status: None,
            error: Some(format!("connect: {e}")),
            via_proxy: false,
        },
        Err(_) => PingResult {
            latency_ms: None,
            status: None,
            error: Some(format!("timeout ({}s)", timeout_dur.as_secs())),
            via_proxy: false,
        },
    }
}

async fn http_ping(
    url: &str,
    method: &str,
    socks_port: Option<u16>,
    timeout_dur: Duration,
) -> PingResult {
    let via_proxy = socks_port.is_some();
    let client = match build_client(socks_port, timeout_dur) {
        Ok(c) => c,
        Err(e) => {
            return PingResult {
                latency_ms: None,
                status: None,
                error: Some(format!("client: {e:#}")),
                via_proxy,
            };
        }
    };

    let req = match method {
        "GET" => client.get(url),
        "HEAD" => client.head(url),
        other => {
            return PingResult {
                latency_ms: None,
                status: None,
                error: Some(format!("unsupported method: {other}")),
                via_proxy,
            };
        }
    };

    let start = Instant::now();
    let result = req.send().await;
    let elapsed_ms = start.elapsed().as_millis() as u32;
    match result {
        Ok(resp) => PingResult {
            latency_ms: Some(elapsed_ms),
            status: Some(resp.status().as_u16()),
            error: None,
            via_proxy,
        },
        Err(e) => PingResult {
            latency_ms: None,
            status: None,
            error: Some(format!("{e}")),
            via_proxy,
        },
    }
}

fn build_client(socks_port: Option<u16>, timeout_dur: Duration) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(timeout_dur)
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/130.0 Safari/537.36",
        );
    if let Some(port) = socks_port {
        let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}"))
            .context("invalid socks5 URL")?;
        builder = builder.proxy(proxy);
    }
    builder.build().context("reqwest client build")
}

/// Парсинг URL для TCP-ping: возвращает (host, port).
/// Поддерживает форматы:
///   - `https://example.com[:443][/path]` → ("example.com", 443)
///   - `http://example.com[:80][/path]` → ("example.com", 80)
///   - `example.com:8080` → ("example.com", 8080)
///   - `example.com` → ("example.com", 443) // дефолт https
fn parse_host_port(url: &str) -> Result<(String, u16)> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        bail!("пустой URL");
    }

    // Снять схему если есть.
    let (scheme_default_port, after_scheme) = if let Some(rest) = trimmed.strip_prefix("https://") {
        (443u16, rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        (80u16, rest)
    } else {
        (443u16, trimmed)
    };

    // Отрезать path (всё после первого `/`).
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    if host_port.is_empty() {
        return Err(anyhow!("URL без host"));
    }

    // Разобрать host[:port].
    if let Some(colon) = host_port.rfind(':') {
        // IPv6 в квадратных скобках: [::1]:443. Простая эвристика.
        if host_port.starts_with('[') {
            if let Some(end) = host_port.find(']') {
                let host = host_port[1..end].to_string();
                let port = if let Some(p) = host_port[end + 1..].strip_prefix(':') {
                    p.parse::<u16>().context("invalid port")?
                } else {
                    scheme_default_port
                };
                return Ok((host, port));
            }
        }
        let host = host_port[..colon].to_string();
        let port = host_port[colon + 1..]
            .parse::<u16>()
            .context("invalid port")?;
        if host.is_empty() {
            bail!("URL без host");
        }
        Ok((host, port))
    } else {
        Ok((host_port.to_string(), scheme_default_port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_url() {
        let (h, p) = parse_host_port("https://www.gstatic.com/generate_204").unwrap();
        assert_eq!(h, "www.gstatic.com");
        assert_eq!(p, 443);
    }

    #[test]
    fn parse_with_port() {
        let (h, p) = parse_host_port("https://example.com:8443/foo").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 8443);
    }

    #[test]
    fn parse_http_default_port() {
        let (h, p) = parse_host_port("http://example.com/").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 80);
    }

    #[test]
    fn parse_no_scheme() {
        let (h, p) = parse_host_port("example.com").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 443);
    }

    #[test]
    fn parse_no_scheme_with_port() {
        let (h, p) = parse_host_port("example.com:8080").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 8080);
    }

    #[test]
    fn parse_empty_fails() {
        assert!(parse_host_port("").is_err());
        assert!(parse_host_port("   ").is_err());
    }

    #[test]
    fn parse_invalid_port_fails() {
        assert!(parse_host_port("example.com:abc").is_err());
    }
}
