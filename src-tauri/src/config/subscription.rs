//! Загрузка и парсинг подписки.
//!
//! Основной формат — base64-список URI (vless://, ss://, vmess://, trojan://).
//! Fallback — Clash YAML (если сервер вернул его вместо base64).

use std::sync::Mutex;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

use super::server::ProxyEntry;

/// Метаданные подписки из заголовка `subscription-userinfo`
/// (де-факто стандарт у 3x-ui / Marzban / x-ui / sing-box).
///
/// Формат заголовка: `upload=X;download=Y;total=Z;expire=T`,
/// где X/Y/Z — байты (Z=0 → безлимит), T — unix-timestamp срока
/// истечения (T=0 → бессрочно).
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionMeta {
    /// upload + download в байтах.
    pub used: u64,
    /// Общий лимит в байтах. 0 = безлимит.
    pub total: u64,
    /// Unix-timestamp истечения. None = бессрочно.
    pub expire_at: Option<i64>,
}

/// Кешированный список серверов и метаданных из последней успешной
/// загрузки подписки. Живёт в памяти процесса, не персистится между
/// запусками (для этого есть localStorage на фронте).
pub struct SubscriptionState {
    pub servers: Mutex<Vec<ProxyEntry>>,
    pub meta: Mutex<Option<SubscriptionMeta>>,
}

impl SubscriptionState {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(Vec::new()),
            meta: Mutex::new(None),
        }
    }
}

/// Парсит заголовок `subscription-userinfo` вида
/// `upload=123;download=456;total=789;expire=1700000000`.
/// Неизвестные ключи игнорируются, отсутствующие → 0.
pub fn parse_subscription_userinfo(raw: &str) -> SubscriptionMeta {
    let mut upload: u64 = 0;
    let mut download: u64 = 0;
    let mut total: u64 = 0;
    let mut expire: i64 = 0;

    for pair in raw.split(';') {
        let pair = pair.trim();
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "upload" => upload = v.parse().unwrap_or(0),
            "download" => download = v.parse().unwrap_or(0),
            "total" => total = v.parse().unwrap_or(0),
            "expire" => expire = v.parse().unwrap_or(0),
            _ => {}
        }
    }

    SubscriptionMeta {
        used: upload.saturating_add(download),
        total,
        expire_at: if expire > 0 { Some(expire) } else { None },
    }
}

/// Скачать подписку по URL и вернуть список серверов.
///
/// `user_agent` — UA для запроса. По умолчанию `Happ/2.7.0` (так провайдеры
/// на базе Marzban / RemnaWave отдают массив готовых Xray-конфигов).
/// `hwid` — идентификатор устройства, шлётся в заголовке `x-hwid`. Сервер
/// регистрирует новое устройство автоматически, если в подписке есть
/// свободный HWID-слот. Если `send_hwid=false`, заголовок не шлётся.
pub async fn fetch_and_parse(
    url: &str,
    hwid: &str,
    user_agent: &str,
    send_hwid: bool,
) -> Result<(Vec<ProxyEntry>, Option<SubscriptionMeta>)> {
    let ua = if user_agent.trim().is_empty() {
        "Happ/2.7.0"
    } else {
        user_agent
    };

    let client = reqwest::Client::builder()
        .user_agent(ua)
        .build()
        .context("не удалось создать HTTP-клиент")?;

    let mut req = client.get(url);
    if send_hwid && !hwid.is_empty() {
        req = req.header("x-hwid", hwid);
    }

    let response = req
        .send()
        .await
        .context("ошибка HTTP-запроса")?
        .error_for_status()
        .context("сервер вернул ошибку")?;

    // Извлекаем метаданные из заголовков ДО чтения body (после `text()`
    // response уже потреблён). subscription-userinfo — единственный
    // стандартный заголовок, остальные подключим позже (этап 8.C).
    let meta = response
        .headers()
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(parse_subscription_userinfo);

    let body = response
        .text()
        .await
        .context("не удалось прочитать тело ответа")?;

    let servers = parse_subscription_body(&body)?;
    Ok((servers, meta))
}

/// Парсит тело подписки, перебирая известные форматы по приоритету.
fn parse_subscription_body(body: &str) -> Result<Vec<ProxyEntry>> {
    // 1. Xray JSON конфиг — приоритетнее всего, чтобы случайно
    //    не распарсить JSON как base64. Может быть как одиночным объектом,
    //    так и массивом (Happ-формат подписки).
    let head = body.trim_start();
    if head.starts_with('{') || head.starts_with('[') {
        if let Ok(entries) = parse_xray_json(body) {
            if !entries.is_empty() {
                return Ok(entries);
            }
        }
    }

    // 2. base64-список URI
    if let Ok(entries) = parse_base64_uri_list(body) {
        if !entries.is_empty() {
            return Ok(entries);
        }
    }

    // 3. Plain text URI list (по одному URI на строку)
    if let Ok(entries) = parse_plain_uri_list(body) {
        if !entries.is_empty() {
            return Ok(entries);
        }
    }

    // 4. Fallback: Clash YAML
    parse_clash_yaml(body)
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

// ─── plain text URI list ──────────────────────────────────────────────────────

fn parse_plain_uri_list(text: &str) -> Result<Vec<ProxyEntry>> {
    let entries: Vec<ProxyEntry> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| parse_proxy_uri(l).ok())
        .collect();

    if entries.is_empty() {
        bail!("нет URI в plain text");
    }
    Ok(entries)
}

// ─── Xray JSON конфиг as-is ───────────────────────────────────────────────────

/// Парсит Xray JSON: либо одиночный объект-конфиг, либо массив таких объектов.
/// Каждый конфиг становится отдельным ProxyEntry с name = `remarks`.
fn parse_xray_json(text: &str) -> Result<Vec<ProxyEntry>> {
    let json: serde_json::Value =
        serde_json::from_str(text).context("не удалось распарсить Xray JSON")?;

    let configs: Vec<serde_json::Value> = match json {
        serde_json::Value::Array(arr) => arr,
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => bail!("Xray JSON: ожидался объект или массив объектов"),
    };

    let entries: Vec<ProxyEntry> = configs
        .into_iter()
        .filter(|c| c.get("outbounds").is_some() || c.get("inbounds").is_some())
        .enumerate()
        .map(|(i, cfg)| {
            let name = cfg["remarks"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Xray config #{}", i + 1));
            ProxyEntry {
                name,
                protocol: "xray-json".to_string(),
                server: "127.0.0.1".to_string(),
                port: 0,
                raw: cfg,
            }
        })
        .collect();

    if entries.is_empty() {
        bail!("в Xray JSON нет ни одного конфига с inbounds/outbounds");
    }
    Ok(entries)
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
