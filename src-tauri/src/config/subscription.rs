//! Загрузка и парсинг подписки.
//!
//! Основной формат — base64-список URI (vless://, ss://, vmess://, trojan://).
//! Fallback — Clash YAML (если сервер вернул его вместо base64).

use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;

use super::server::ProxyEntry;

/// Кешированный список серверов из последней успешной загрузки подписки.
pub struct SubscriptionState {
    pub servers: Mutex<Vec<ProxyEntry>>,
}

impl SubscriptionState {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(Vec::new()),
        }
    }
}

/// Скачать подписку по URL и вернуть список серверов.
pub async fn fetch_and_parse(url: &str, hwid: &str) -> Result<Vec<ProxyEntry>> {
    let client = reqwest::Client::builder()
        .user_agent("NemefistoVPN/1.0")
        .build()
        .context("не удалось создать HTTP-клиент")?;

    let body = client
        .get(url)
        .header("x-hwid", hwid)
        .send()
        .await
        .context("ошибка HTTP-запроса")?
        .error_for_status()
        .context("сервер вернул ошибку")?
        .text()
        .await
        .context("не удалось прочитать тело ответа")?;

    // Основной путь: base64-список URI
    if let Ok(entries) = parse_base64_uri_list(&body) {
        if !entries.is_empty() {
            return Ok(entries);
        }
    }

    // Fallback: Clash YAML
    parse_clash_yaml(&body)
}

// ─── base64 URI-список ────────────────────────────────────────────────────────

fn parse_base64_uri_list(text: &str) -> Result<Vec<ProxyEntry>> {
    let trimmed = text.trim();
    let decoded = general_purpose::STANDARD
        .decode(trimmed)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(trimmed))
        .or_else(|_| general_purpose::URL_SAFE.decode(trimmed))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(trimmed))
        .context("не base64")?;

    let text = String::from_utf8(decoded).context("декодированный текст — не UTF-8")?;

    let entries: Vec<ProxyEntry> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| parse_proxy_uri(l).ok())
        .collect();

    if entries.is_empty() {
        bail!("пустой список URI");
    }
    Ok(entries)
}

fn parse_proxy_uri(uri: &str) -> Result<ProxyEntry> {
    if uri.starts_with("vless://") {
        parse_vless(uri)
    } else if uri.starts_with("vmess://") {
        parse_vmess(uri)
    } else if uri.starts_with("trojan://") {
        parse_trojan(uri)
    } else if uri.starts_with("ss://") {
        parse_ss(uri)
    } else {
        bail!("неизвестный протокол: {uri}")
    }
}

// ─── парсеры URI ──────────────────────────────────────────────────────────────

fn parse_vless(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("vless://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (uuid, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в VLESS URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("uuid".into(), uuid.to_string().into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "vless".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
    })
}

fn parse_vmess(uri: &str) -> Result<ProxyEntry> {
    let b64 = uri.strip_prefix("vmess://").unwrap().trim();
    let decoded = general_purpose::STANDARD
        .decode(b64)
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(b64))
        .context("не удалось декодировать VMess base64")?;

    let json: serde_json::Value =
        serde_json::from_slice(&decoded).context("VMess JSON невалиден")?;

    let name = json["ps"].as_str().unwrap_or("VMess").to_string();
    let server = json["add"]
        .as_str()
        .context("поле add обязательно")?
        .to_string();
    let port: u16 = json["port"]
        .as_u64()
        .or_else(|| json["port"].as_str().and_then(|s| s.parse().ok()))
        .context("поле port обязательно")? as u16;

    Ok(ProxyEntry {
        name,
        protocol: "vmess".to_string(),
        server,
        port,
        raw: json,
    })
}

fn parse_trojan(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("trojan://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (password, host, port) = split_userinfo_hostport(authority)
        .context("некорректный authority в Trojan URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("password".into(), password.to_string().into());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            raw.insert(k.to_string(), url_decode(v).into());
        }
    }

    Ok(ProxyEntry {
        name,
        protocol: "trojan".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
    })
}

fn parse_ss(uri: &str) -> Result<ProxyEntry> {
    let rest = uri.strip_prefix("ss://").unwrap();

    let (rest, name) = split_fragment(rest);
    let (rest, _query) = split_query(rest);

    let (userinfo_b64, host, port) =
        split_userinfo_hostport(rest).context("некорректный SS URI")?;

    let userinfo_bytes = general_purpose::STANDARD
        .decode(userinfo_b64)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(userinfo_b64))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(userinfo_b64))
        .context("не удалось декодировать base64 userinfo в SS URI")?;

    let userinfo = String::from_utf8(userinfo_bytes)?;
    let (cipher, password) = userinfo
        .split_once(':')
        .context("некорректный userinfo в SS URI")?;

    let mut raw = serde_json::Map::new();
    raw.insert("cipher".into(), cipher.to_string().into());
    raw.insert("password".into(), password.to_string().into());

    Ok(ProxyEntry {
        name,
        protocol: "ss".to_string(),
        server: host.to_string(),
        port,
        raw: serde_json::Value::Object(raw),
    })
}

// ─── вспомогательные функции ──────────────────────────────────────────────────

/// `"...#fragment"` → `("...", decoded_name)`
fn split_fragment(s: &str) -> (&str, String) {
    match s.rfind('#') {
        Some(i) => (&s[..i], url_decode(&s[i + 1..])),
        None => (s, "Unknown".to_string()),
    }
}

/// `"authority?query"` → `("authority", "query")`
fn split_query(s: &str) -> (&str, &str) {
    match s.find('?') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

/// `"user@host:port"` → `(user, host, port)`
fn split_userinfo_hostport(s: &str) -> Option<(&str, &str, u16)> {
    let at = s.rfind('@')?;
    let userinfo = &s[..at];
    let host_port = &s[at + 1..];

    let (host, port_str) = if host_port.starts_with('[') {
        // IPv6: [::1]:443
        let close = host_port.find(']')?;
        let port_str = host_port[close + 1..].strip_prefix(':')?;
        (&host_port[..=close], port_str)
    } else {
        let colon = host_port.rfind(':')?;
        (&host_port[..colon], &host_port[colon + 1..])
    };

    let port: u16 = port_str.parse().ok()?;
    Some((userinfo, host, port))
}

/// Декодирует URL-encoding (%XX), включая многобайтовые UTF-8 последовательности.
fn url_decode(s: &str) -> String {
    let bytes_in = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes_in.len());
    let mut i = 0;
    while i < bytes_in.len() {
        if bytes_in[i] == b'%' && i + 3 <= bytes_in.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes_in[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        } else if bytes_in[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes_in[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

// ─── Clash YAML fallback ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ClashConfig {
    #[serde(default)]
    proxies: Vec<serde_yaml::Value>,
}

fn parse_clash_yaml(text: &str) -> Result<Vec<ProxyEntry>> {
    let config: ClashConfig =
        serde_yaml::from_str(text).context("не удалось распарсить Clash YAML")?;

    let entries = config
        .proxies
        .into_iter()
        .filter_map(|v| yaml_proxy_to_entry(v).ok())
        .collect();

    Ok(entries)
}

fn yaml_proxy_to_entry(v: serde_yaml::Value) -> Result<ProxyEntry> {
    let map = v.as_mapping().context("proxy-запись — не mapping")?;

    let name = map
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let protocol = map
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let server = map
        .get("server")
        .and_then(|v| v.as_str())
        .context("поле server обязательно")?
        .to_string();
    let port = map
        .get("port")
        .and_then(|v| v.as_u64())
        .context("поле port обязательно")? as u16;

    let raw = serde_json::to_value(&v)
        .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

    Ok(ProxyEntry {
        name,
        protocol,
        server,
        port,
        raw,
    })
}
